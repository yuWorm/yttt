//! Bounded decoding for Kitty graphics APC commands.
//!
//! This module deliberately does not keep an image until every chunk and every
//! decoder stage has succeeded.  The state layer can therefore treat a returned
//! image as an atomic transmission.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::Arc;

use base64::Engine as _;
use flate2::read::ZlibDecoder;

const MAX_APC_BYTES: usize = 8 * 1024;
const MAX_ENCODED_BYTES: usize = 2 * crate::graphics::MAX_GRAPHIC_BYTES;
const MAX_RAW_BYTES: usize = crate::graphics::MAX_GRAPHIC_BYTES;
const MAX_DIMENSION: u32 = 4096;
const MAX_NAME_BYTES: usize = 4096;

/// Parsed Kitty graphics controls.
#[derive(Clone, Debug, Default)]
pub struct Controls {
    values: BTreeMap<u8, i64>,
}

impl Controls {
    /// Parse a comma-separated Kitty graphics control header.
    pub fn parse(header: &[u8]) -> Result<Self, ProtocolError> {
        match Self::parse_inner(header) {
            Ok(controls) => Ok(controls),
            Err(mut error) => {
                for (key, value) in response_controls(header).values {
                    error.controls.values.entry(key).or_insert(value);
                }
                Err(error)
            }
        }
    }

    fn parse_inner(header: &[u8]) -> Result<Self, ProtocolError> {
        let mut controls = Self::default();
        if header.is_empty() {
            return Ok(controls);
        }

        for field in header.split(|byte| *byte == b',') {
            let Some((key, value)) = field
                .split_first()
                .and_then(|(key, rest)| rest.strip_prefix(b"=").map(|value| (*key, value)))
            else {
                return Err(ProtocolError::new(
                    "EINVAL",
                    "invalid graphics control field",
                    controls,
                ));
            };

            if controls.values.contains_key(&key) {
                return Err(ProtocolError::new(
                    "EINVAL",
                    format!("duplicate graphics control {}", key as char),
                    controls,
                ));
            }

            let parsed = match key {
                b'a' => parse_enum(value, b"acdfpqtT", "action", &controls)?,
                b't' => parse_enum(value, b"dfts", "transmission medium", &controls)?,
                b'o' => parse_enum(value, b"z", "compression", &controls)?,
                b'd' => parse_enum(value, b"aAcCnNiIpPqQrRxXyYzZ", "delete selector", &controls)?,
                b'z' | b'H' | b'V' => parse_signed(value, key, &controls)?,
                b'q' => {
                    let value = parse_unsigned(value, key, &controls)?;
                    if value > 2 {
                        return Err(ProtocolError::new(
                            "EINVAL",
                            "q must be 0, 1, or 2",
                            controls,
                        ));
                    }
                    value
                }
                b'f' => {
                    let value = parse_unsigned(value, key, &controls)?;
                    if !matches!(value, 24 | 32 | 100) {
                        return Err(ProtocolError::new(
                            "EINVAL",
                            "unsupported pixel format",
                            controls,
                        ));
                    }
                    value
                }
                b'm' => {
                    let value = parse_unsigned(value, key, &controls)?;
                    if value > 1 {
                        return Err(ProtocolError::new("EINVAL", "m must be 0 or 1", controls));
                    }
                    value
                }
                b's' | b'v' | b'S' | b'O' | b'i' | b'I' | b'p' | b'N' | b'x' | b'y' | b'w'
                | b'h' | b'X' | b'Y' | b'c' | b'r' | b'C' | b'U' | b'P' | b'Q' => {
                    parse_unsigned(value, key, &controls)?
                }
                _ => {
                    return Err(ProtocolError::new(
                        "EINVAL",
                        format!("unknown graphics control {}", key as char),
                        controls,
                    ));
                }
            };
            controls.values.insert(key, parsed);
        }

        Ok(controls)
    }

    pub fn get(&self, key: u8) -> Option<i64> {
        self.values.get(&key).copied()
    }

    pub fn unsigned(&self, key: u8, default: u32) -> u32 {
        self.get(key).map_or(default, |value| value as u32)
    }

    pub fn signed(&self, key: u8, default: i32) -> i32 {
        self.get(key).map_or(default, |value| value as i32)
    }

    pub fn byte(&self, key: u8, default: u8) -> u8 {
        self.get(key).map_or(default, |value| value as u8)
    }

    /// Set a parsed control, used when a final chunk supplies its response mode.
    pub fn set(&mut self, key: u8, value: i64) {
        self.values.insert(key, value);
    }

    fn contains(&self, key: u8) -> bool {
        self.values.contains_key(&key)
    }

    fn keys(&self) -> impl Iterator<Item = u8> + '_ {
        self.values.keys().copied()
    }
}

fn response_controls(header: &[u8]) -> Controls {
    let mut controls = Controls::default();
    for field in header.split(|byte| *byte == b',') {
        let Some((key, value)) = field
            .split_first()
            .and_then(|(key, rest)| rest.strip_prefix(b"=").map(|value| (*key, value)))
        else {
            continue;
        };
        match key {
            b'i' | b'I' | b'p' | b'q'
                if !value.is_empty() && value.iter().all(u8::is_ascii_digit) =>
            {
                if let Some(value) = std::str::from_utf8(value)
                    .ok()
                    .and_then(|value| value.parse::<u32>().ok())
                {
                    if key != b'q' || value <= 2 {
                        controls.set(key, i64::from(value));
                    }
                }
            }
            b'a' | b't' | b'o' | b'd' if value.len() == 1 => {
                controls.set(key, i64::from(value[0]));
            }
            _ => {}
        }
    }
    controls
}

fn parse_unsigned(value: &[u8], key: u8, controls: &Controls) -> Result<i64, ProtocolError> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return Err(ProtocolError::new(
            "EINVAL",
            format!("{} must be an unsigned 32-bit integer", key as char),
            controls.clone(),
        ));
    }

    let value = std::str::from_utf8(value)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| {
            ProtocolError::new(
                "EINVAL",
                format!("{} is outside the unsigned 32-bit range", key as char),
                controls.clone(),
            )
        })?;
    Ok(i64::from(value))
}

fn parse_signed(value: &[u8], key: u8, controls: &Controls) -> Result<i64, ProtocolError> {
    let digits = value.strip_prefix(b"-").unwrap_or(value);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(ProtocolError::new(
            "EINVAL",
            format!("{} must be a signed 32-bit integer", key as char),
            controls.clone(),
        ));
    }

    let value = std::str::from_utf8(value)
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or_else(|| {
            ProtocolError::new(
                "EINVAL",
                format!("{} is outside the signed 32-bit range", key as char),
                controls.clone(),
            )
        })?;
    Ok(i64::from(value))
}

fn parse_enum(
    value: &[u8],
    allowed: &[u8],
    label: &str,
    controls: &Controls,
) -> Result<i64, ProtocolError> {
    let Some(&value) = value.first().filter(|_| value.len() == 1) else {
        return Err(ProtocolError::new(
            "EINVAL",
            format!("{label} must be one character"),
            controls.clone(),
        ));
    };
    if !allowed.contains(&value) {
        return Err(ProtocolError::new(
            "EINVAL",
            format!("unsupported {label}"),
            controls.clone(),
        ));
    }
    Ok(i64::from(value))
}

/// A decoding failure that can be turned into a Kitty response by the state layer.
#[derive(Debug)]
pub struct ProtocolError {
    pub code: &'static str,
    pub message: String,
    pub controls: Controls,
}

impl ProtocolError {
    pub fn new(code: &'static str, message: impl Into<String>, controls: Controls) -> Self {
        Self {
            code,
            message: message.into(),
            controls,
        }
    }
}

/// A fully decoded, immutable RGBA image.
#[derive(Clone, Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

/// A Kitty command after image payload decoding, if the action carries an image.
#[derive(Debug)]
pub struct DecodedCommand {
    pub controls: Controls,
    pub image: Option<DecodedImage>,
}

#[derive(Debug)]
struct PendingUpload {
    controls: Controls,
    encoded: Vec<u8>,
}

/// Decoder for ordered Kitty APC sequences.
#[derive(Debug, Default)]
pub struct Decoder {
    pending: Option<PendingUpload>,
}

impl Decoder {
    /// Decode one APC body. `apc` begins with `G`, without ESC/APC framing.
    pub fn decode(
        &mut self,
        apc: &[u8],
        allow_files: bool,
    ) -> Result<Option<DecodedCommand>, ProtocolError> {
        if apc.first() != Some(&b'G') {
            self.abort();
            return Err(ProtocolError::new(
                "EINVAL",
                "graphics APC must begin with G",
                Controls::default(),
            ));
        }

        let body = &apc[1..];
        let (header, payload) = body
            .iter()
            .position(|byte| *byte == b';')
            .map_or((body, &[][..]), |separator| {
                (&body[..separator], &body[separator + 1..])
            });
        let controls = match Controls::parse(header) {
            Ok(controls) => controls,
            Err(error) => {
                self.abort();
                return Err(error);
            }
        };

        if apc.len() > MAX_APC_BYTES {
            self.abort();
            return Err(ProtocolError::new(
                "E2BIG",
                "graphics APC exceeds the bounded control buffer",
                controls,
            ));
        }
        if apc.contains(&0) {
            self.abort();
            return Err(ProtocolError::new(
                "EINVAL",
                "NUL is not valid in a graphics APC",
                controls,
            ));
        }

        let action = controls.byte(b'a', b't');
        if action == b'd' {
            // The protocol explicitly makes delete a cancellation boundary.
            self.abort();
            if !payload.is_empty() {
                return Err(ProtocolError::new(
                    "EINVAL",
                    "delete commands cannot carry image data",
                    controls,
                ));
            }
            return Ok(Some(DecodedCommand {
                controls,
                image: None,
            }));
        }

        if self.pending.is_some() {
            return self.decode_continuation(controls, payload);
        }

        if !image_action(action) {
            if controls.unsigned(b'm', 0) != 0 || !payload.is_empty() {
                return Err(ProtocolError::new(
                    "EINVAL",
                    "this graphics action cannot carry image data",
                    controls,
                ));
            }
            return Ok(Some(DecodedCommand {
                controls,
                image: None,
            }));
        }

        let medium = controls.byte(b't', b'd');
        if medium != b'd' && !allow_files {
            return Err(ProtocolError::new(
                "EACCES",
                "file and shared-memory transfers are disabled",
                controls,
            ));
        }

        if medium == b'd' {
            if controls.unsigned(b'm', 0) == 1 && payload.len() % 4 != 0 {
                return Err(ProtocolError::new(
                    "EINVAL",
                    "non-final graphics chunk is not base64-aligned",
                    controls,
                ));
            }
            let mut encoded = Vec::new();
            append_base64(
                payload,
                &mut encoded,
                source_input_limit(&controls)?,
                &controls,
            )?;
            if controls.unsigned(b'm', 0) == 1 {
                self.pending = Some(PendingUpload { controls, encoded });
                return Ok(None);
            }
            return decode_complete(controls, encoded).map(Some);
        }

        if controls.unsigned(b'm', 0) != 0 {
            return Err(ProtocolError::new(
                "EINVAL",
                "only direct transfers may be chunked",
                controls,
            ));
        }

        let source = decode_name(payload, &controls)?;
        let bytes = match medium {
            b'f' => read_regular_file(&source, false, &controls)?,
            b't' => read_regular_file(&source, true, &controls)?,
            b's' => read_shared_memory(&source, &controls)?,
            _ => unreachable!("Controls validates the transmission medium"),
        };
        decode_complete(controls, bytes).map(Some)
    }

    /// Discard a partial direct upload.
    pub fn abort(&mut self) {
        self.pending = None;
    }

    fn decode_continuation(
        &mut self,
        continuation: Controls,
        payload: &[u8],
    ) -> Result<Option<DecodedCommand>, ProtocolError> {
        // Taking first guarantees that any malformed continuation cannot leave a
        // partial image to be accidentally finished by a later command.
        let PendingUpload {
            mut controls,
            mut encoded,
        } = self.pending.take().expect("checked above");
        if let Some(value) = continuation.get(b'q') {
            controls.set(b'q', value);
        }

        let initial_action = controls.byte(b'a', b't');
        let continuation_is_valid = continuation.contains(b'm')
            && continuation
                .keys()
                .all(|key| matches!(key, b'm' | b'q' | b'a'))
            && match continuation.get(b'a') {
                Some(value) => initial_action == b'f' && value == i64::from(b'f'),
                None => initial_action != b'f',
            };
        if !continuation_is_valid {
            return Err(ProtocolError::new(
                "EINVAL",
                "invalid graphics continuation controls",
                controls,
            ));
        }

        if continuation.unsigned(b'm', 0) == 1 && payload.len() % 4 != 0 {
            return Err(ProtocolError::new(
                "EINVAL",
                "non-final graphics chunk is not base64-aligned",
                controls,
            ));
        }
        append_base64(
            payload,
            &mut encoded,
            source_input_limit(&controls)?,
            &controls,
        )?;

        if continuation.unsigned(b'm', 0) == 1 {
            self.pending = Some(PendingUpload { controls, encoded });
            return Ok(None);
        }

        decode_complete(controls, encoded).map(Some)
    }
}

fn image_action(action: u8) -> bool {
    matches!(action, b't' | b'T' | b'q' | b'f')
}

fn source_input_limit(controls: &Controls) -> Result<usize, ProtocolError> {
    let format = controls.unsigned(b'f', 32);
    if controls.byte(b'o', 0) == b'z' && format == 100 {
        let declared = controls.unsigned(b'S', 0) as usize;
        if declared == 0 {
            return Err(ProtocolError::new(
                "ENODATA",
                "compressed PNG transmissions require S",
                controls.clone(),
            ));
        }
        if declared > MAX_ENCODED_BYTES {
            return Err(ProtocolError::new(
                "ENOSPC",
                "decompressed PNG source exceeds the decoder limit",
                controls.clone(),
            ));
        }
        Ok(MAX_ENCODED_BYTES)
    } else if controls.byte(b'o', 0) == b'z' || format == 100 {
        Ok(MAX_ENCODED_BYTES)
    } else {
        expected_raw_len(controls, format)
    }
}

fn append_base64(
    payload: &[u8],
    destination: &mut Vec<u8>,
    maximum: usize,
    controls: &Controls,
) -> Result<(), ProtocolError> {
    if payload.contains(&0) {
        return Err(ProtocolError::new(
            "EINVAL",
            "NUL is not valid in base64 graphics data",
            controls.clone(),
        ));
    }
    let maximum_added = payload
        .len()
        .checked_add(3)
        .and_then(|length| length.checked_div(4))
        .and_then(|quads| quads.checked_mul(3))
        .and_then(|length| {
            let padding = payload
                .iter()
                .rev()
                .take(2)
                .take_while(|byte| **byte == b'=')
                .count();
            length.checked_sub(padding)
        })
        .ok_or_else(|| {
            ProtocolError::new(
                "E2BIG",
                "base64 graphics data is too large",
                controls.clone(),
            )
        })?;
    if destination
        .len()
        .checked_add(maximum_added)
        .is_none_or(|length| length > maximum)
    {
        return Err(ProtocolError::new(
            "ENOSPC",
            "graphics payload exceeds the encoded-data limit",
            controls.clone(),
        ));
    }

    base64::engine::general_purpose::STANDARD
        .decode_vec(payload, destination)
        .map_err(|_| {
            ProtocolError::new(
                "EINVAL",
                "graphics payload is not valid base64",
                controls.clone(),
            )
        })?;
    if destination.len() > maximum {
        return Err(ProtocolError::new(
            "ENOSPC",
            "graphics payload exceeds the encoded-data limit",
            controls.clone(),
        ));
    }
    Ok(())
}

fn decode_name(payload: &[u8], controls: &Controls) -> Result<Vec<u8>, ProtocolError> {
    let mut name = Vec::new();
    append_base64(payload, &mut name, MAX_NAME_BYTES, controls)?;
    if name.is_empty() || name.len() > MAX_NAME_BYTES || name.contains(&0) {
        return Err(ProtocolError::new(
            "EINVAL",
            "invalid graphics file or shared-memory name",
            controls.clone(),
        ));
    }
    Ok(name)
}

fn decode_complete(controls: Controls, bytes: Vec<u8>) -> Result<DecodedCommand, ProtocolError> {
    let bytes = if controls.byte(b'o', 0) == b'z' {
        decompress(&bytes, &controls)?
    } else {
        bytes
    };
    let image = decode_pixels(&controls, bytes)?;
    Ok(DecodedCommand {
        controls,
        image: Some(image),
    })
}

fn decompress(input: &[u8], controls: &Controls) -> Result<Vec<u8>, ProtocolError> {
    let format = controls.unsigned(b'f', 32);
    let expected = if format == 100 {
        let declared = controls.unsigned(b'S', 0) as usize;
        if declared == 0 {
            return Err(ProtocolError::new(
                "ENODATA",
                "compressed PNG transmissions require S",
                controls.clone(),
            ));
        }
        declared
    } else {
        expected_raw_len(controls, format)?
    };
    if expected > MAX_ENCODED_BYTES {
        return Err(ProtocolError::new(
            "ENOSPC",
            "decompressed graphics data exceeds the input limit",
            controls.clone(),
        ));
    }

    let mut output = Vec::with_capacity(expected);
    let mut decoder = ZlibDecoder::new(input);
    decoder
        .by_ref()
        .take(expected.saturating_add(1) as u64)
        .read_to_end(&mut output)
        .map_err(|error| io_error(error, controls))?;
    if output.len() != expected {
        return Err(ProtocolError::new(
            "EINVAL",
            "compressed graphics data has an unexpected length",
            controls.clone(),
        ));
    }
    Ok(output)
}

fn decode_pixels(controls: &Controls, bytes: Vec<u8>) -> Result<DecodedImage, ProtocolError> {
    match controls.unsigned(b'f', 32) {
        24 | 32 => decode_raw_pixels(controls, bytes),
        100 => decode_png(controls, bytes),
        _ => Err(ProtocolError::new(
            "EINVAL",
            "unsupported pixel format",
            controls.clone(),
        )),
    }
}

fn expected_raw_len(controls: &Controls, format: u32) -> Result<usize, ProtocolError> {
    let width = controls.unsigned(b's', 0);
    let height = controls.unsigned(b'v', 0);
    validate_dimensions(width, height, controls)?;
    let channels = match format {
        24 => 3usize,
        32 => 4usize,
        _ => unreachable!("only raw formats call expected_raw_len"),
    };
    let bytes = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(height).ok()?))
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or_else(|| {
            ProtocolError::new(
                "ENOSPC",
                "raw graphics dimensions overflow",
                controls.clone(),
            )
        })?;
    if bytes > MAX_RAW_BYTES {
        return Err(ProtocolError::new(
            "ENOSPC",
            "raw graphics pixels exceed the decoder limit",
            controls.clone(),
        ));
    }
    Ok(bytes)
}

fn validate_dimensions(width: u32, height: u32, controls: &Controls) -> Result<(), ProtocolError> {
    if width == 0 || height == 0 {
        return Err(ProtocolError::new(
            "ENODATA",
            "raw graphics transmissions require s and v",
            controls.clone(),
        ));
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(ProtocolError::new(
            "ENOSPC",
            "graphics dimensions exceed 4096 pixels",
            controls.clone(),
        ));
    }
    let rgba_len = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(height).ok()?))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            ProtocolError::new("ENOSPC", "graphics dimensions overflow", controls.clone())
        })?;
    if rgba_len > MAX_RAW_BYTES {
        return Err(ProtocolError::new(
            "ENOSPC",
            "decoded RGBA pixels exceed the decoder limit",
            controls.clone(),
        ));
    }
    Ok(())
}

fn decode_raw_pixels(controls: &Controls, bytes: Vec<u8>) -> Result<DecodedImage, ProtocolError> {
    let format = controls.unsigned(b'f', 32);
    let expected = expected_raw_len(controls, format)?;
    if bytes.len() != expected {
        return Err(ProtocolError::new(
            "EINVAL",
            "raw graphics data length does not match its dimensions",
            controls.clone(),
        ));
    }
    let width = controls.unsigned(b's', 0);
    let height = controls.unsigned(b'v', 0);

    let rgba = if format == 32 {
        bytes
    } else {
        let mut rgba = Vec::with_capacity(
            usize::try_from(width)
                .unwrap_or_default()
                .saturating_mul(usize::try_from(height).unwrap_or_default())
                .saturating_mul(4),
        );
        for pixel in bytes.chunks_exact(3) {
            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
        rgba
    };
    Ok(DecodedImage {
        width,
        height,
        rgba: Arc::new(rgba),
    })
}

fn decode_png(controls: &Controls, bytes: Vec<u8>) -> Result<DecodedImage, ProtocolError> {
    if bytes.len() > MAX_ENCODED_BYTES {
        return Err(ProtocolError::new(
            "ENOSPC",
            "PNG source exceeds the decoder limit",
            controls.clone(),
        ));
    }

    let mut limits = png::Limits::default();
    limits.bytes = MAX_RAW_BYTES;
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|error| {
        ProtocolError::new(
            "EINVAL",
            format!("invalid PNG data: {error}"),
            controls.clone(),
        )
    })?;
    let info = reader.info();
    validate_dimensions(info.width, info.height, controls)?;

    let output_len = reader.output_buffer_size();
    if output_len > MAX_RAW_BYTES {
        return Err(ProtocolError::new(
            "ENOSPC",
            "PNG output exceeds the decoder limit",
            controls.clone(),
        ));
    }
    let mut output = vec![0; output_len];
    let frame = reader.next_frame(&mut output).map_err(|error| {
        ProtocolError::new(
            "EINVAL",
            format!("invalid PNG data: {error}"),
            controls.clone(),
        )
    })?;
    validate_dimensions(frame.width, frame.height, controls)?;
    output.truncate(frame.buffer_size());

    let rgba = match frame.color_type {
        png::ColorType::Rgba => output,
        png::ColorType::Rgb => expand_png_pixels(&output, 3, false, controls)?,
        png::ColorType::Grayscale => expand_png_pixels(&output, 1, false, controls)?,
        png::ColorType::GrayscaleAlpha => expand_png_pixels(&output, 2, true, controls)?,
        png::ColorType::Indexed => {
            return Err(ProtocolError::new(
                "EINVAL",
                "PNG palette was not expanded",
                controls.clone(),
            ));
        }
    };
    let expected = usize::try_from(frame.width)
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(frame.height).ok()?))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| ProtocolError::new("ENOSPC", "PNG dimensions overflow", controls.clone()))?;
    if rgba.len() != expected || rgba.len() > MAX_RAW_BYTES {
        return Err(ProtocolError::new(
            "EINVAL",
            "PNG pixel data has an unexpected length",
            controls.clone(),
        ));
    }

    Ok(DecodedImage {
        width: frame.width,
        height: frame.height,
        rgba: Arc::new(rgba),
    })
}

fn expand_png_pixels(
    source: &[u8],
    channels: usize,
    has_alpha: bool,
    controls: &Controls,
) -> Result<Vec<u8>, ProtocolError> {
    if source.len() % channels != 0 {
        return Err(ProtocolError::new(
            "EINVAL",
            "PNG scanline data is malformed",
            controls.clone(),
        ));
    }
    let pixels = source.len() / channels;
    let mut rgba =
        Vec::with_capacity(pixels.checked_mul(4).ok_or_else(|| {
            ProtocolError::new("ENOSPC", "PNG pixels overflow", controls.clone())
        })?);
    for pixel in source.chunks_exact(channels) {
        match (channels, has_alpha) {
            (3, false) => rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
            (1, false) => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]),
            (2, true) => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]),
            _ => unreachable!("only supported PNG formats call expand_png_pixels"),
        }
    }
    Ok(rgba)
}

fn io_error(error: std::io::Error, controls: &Controls) -> ProtocolError {
    let code = match error.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::UnexpectedEof => "EINVAL",
        _ => "EIO",
    };
    ProtocolError::new(code, error.to_string(), controls.clone())
}

fn read_region(file: &mut std::fs::File, controls: &Controls) -> Result<Vec<u8>, ProtocolError> {
    let length = file
        .metadata()
        .map_err(|error| io_error(error, controls))?
        .len();
    let offset = u64::from(controls.unsigned(b'O', 0));
    if offset > length {
        return Err(ProtocolError::new(
            "EINVAL",
            "graphics source offset exceeds its length",
            controls.clone(),
        ));
    }
    let available = length - offset;
    let compressed_png = controls.byte(b'o', 0) == b'z' && controls.unsigned(b'f', 32) == 100;
    let requested = if compressed_png {
        available
    } else {
        match controls.unsigned(b'S', 0) {
            0 => available,
            size => u64::from(size),
        }
    };
    let processed = controls.byte(b'o', 0) == b'z' || controls.unsigned(b'f', 32) == 100;
    if !processed && requested > available {
        return Err(ProtocolError::new(
            "EINVAL",
            "graphics source size exceeds its length",
            controls.clone(),
        ));
    }
    // PNG and compressed sources are self-delimiting or decoded against their
    // declared output length. Reading to EOF when S is larger than the encoded
    // source matches Kitty and permits compressed PNG files (where S is the
    // uncompressed PNG length).
    let requested = requested.min(available);
    let requested = usize::try_from(requested).map_err(|_| {
        ProtocolError::new(
            "ENOSPC",
            "graphics source size overflows this platform",
            controls.clone(),
        )
    })?;
    let maximum = source_input_limit(controls)?;
    if requested > maximum {
        return Err(ProtocolError::new(
            "ENOSPC",
            "graphics source exceeds the decoder limit",
            controls.clone(),
        ));
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| io_error(error, controls))?;
    let mut bytes = vec![0; requested];
    file.read_exact(&mut bytes)
        .map_err(|error| io_error(error, controls))?;
    Ok(bytes)
}

#[cfg(unix)]
fn read_regular_file(
    name: &[u8],
    temporary: bool,
    controls: &Controls,
) -> Result<Vec<u8>, ProtocolError> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let path = std::path::PathBuf::from(OsString::from_vec(name.to_vec()));
    let canonical = std::fs::canonicalize(&path).map_err(|error| io_error(error, controls))?;
    if sensitive_path(&canonical) && !(temporary && safe_temporary_path(&canonical)) {
        return Err(ProtocolError::new(
            "EACCES",
            "graphics file is in a sensitive filesystem",
            controls.clone(),
        ));
    }
    // O_NONBLOCK ensures an unexpected FIFO/device cannot stall us before the
    // post-open fstat check rejects it. Opening the canonical path follows links
    // as the protocol requires.
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&canonical)
        .map_err(|error| io_error(error, controls))?;
    if !file
        .metadata()
        .map_err(|error| io_error(error, controls))?
        .file_type()
        .is_file()
    {
        return Err(ProtocolError::new(
            "EBADF",
            "graphics source is not a regular file",
            controls.clone(),
        ));
    }

    let read_result = read_region(&mut file, controls);
    let cleanup_result = if temporary && safe_temporary_path(&canonical) {
        std::fs::remove_file(&canonical).map_err(|error| io_error(error, controls))
    } else {
        Ok(())
    };
    cleanup_result?;
    read_result
}

#[cfg(unix)]
fn sensitive_path(path: &std::path::Path) -> bool {
    ["/proc", "/sys", "/dev"]
        .iter()
        .map(std::path::Path::new)
        .any(|root| path.starts_with(root))
}

#[cfg(unix)]
fn safe_temporary_path(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt as _;

    if !path
        .as_os_str()
        .as_bytes()
        .windows(b"tty-graphics-protocol".len())
        .any(|window| window == b"tty-graphics-protocol")
    {
        return false;
    }

    let mut roots = vec![
        std::env::temp_dir(),
        "/tmp".into(),
        "/var/tmp".into(),
        "/dev/shm".into(),
    ];
    if let Some(tmpdir) = std::env::var_os("TMPDIR") {
        roots.push(tmpdir.into());
    }
    roots
        .into_iter()
        .any(|root| std::fs::canonicalize(root).is_ok_and(|root| path.starts_with(root)))
}

#[cfg(unix)]
fn read_shared_memory(name: &[u8], controls: &Controls) -> Result<Vec<u8>, ProtocolError> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd as _;

    if name.first() != Some(&b'/') || name.len() < 2 || name[1..].contains(&b'/') {
        return Err(ProtocolError::new(
            "EINVAL",
            "POSIX shared-memory names must contain one leading slash",
            controls.clone(),
        ));
    }
    let name = CString::new(name).map_err(|_| {
        ProtocolError::new(
            "EINVAL",
            "invalid POSIX shared-memory name",
            controls.clone(),
        )
    })?;
    // `shm_open` returns a descriptor for the object, so fstat/read below are
    // bound to that object even if another process replaces the name later.
    // POSIX sets FD_CLOEXEC itself; Darwin rejects file-open-only O_CLOEXEC.
    let fd = unsafe { libc::shm_open(name.as_ptr(), libc::O_RDONLY, 0) };
    if fd < 0 {
        return Err(io_error(std::io::Error::last_os_error(), controls));
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    #[cfg(not(target_os = "macos"))]
    let mut file = file;
    #[cfg(target_os = "macos")]
    let read_result = read_macos_shared_memory(&file, controls);
    #[cfg(not(target_os = "macos"))]
    let read_result = read_region(&mut file, controls);
    let unlink_result = if unsafe { libc::shm_unlink(name.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(io_error(std::io::Error::last_os_error(), controls))
    };
    drop(file);
    unlink_result?;
    read_result
}

#[cfg(target_os = "macos")]
fn read_macos_shared_memory(
    file: &std::fs::File,
    controls: &Controls,
) -> Result<Vec<u8>, ProtocolError> {
    use std::os::fd::AsRawFd as _;

    let length = file
        .metadata()
        .map_err(|error| io_error(error, controls))?
        .len();
    let offset = u64::from(controls.unsigned(b'O', 0));
    if offset > length {
        return Err(ProtocolError::new(
            "EINVAL",
            "shared-memory offset exceeds its length",
            controls.clone(),
        ));
    }
    let available = length - offset;
    let compressed_png = controls.byte(b'o', 0) == b'z' && controls.unsigned(b'f', 32) == 100;
    let requested = if compressed_png {
        available
    } else {
        match controls.unsigned(b'S', 0) {
            // Darwin reports page-rounded shm lengths, not the requested size.
            0 if controls.byte(b'o', 0) == 0 && controls.unsigned(b'f', 32) != 100 => {
                source_input_limit(controls)? as u64
            }
            0 => available,
            size => u64::from(size),
        }
    };
    let processed = controls.byte(b'o', 0) == b'z' || controls.unsigned(b'f', 32) == 100;
    if !processed && requested > available {
        return Err(ProtocolError::new(
            "EINVAL",
            "shared-memory size exceeds its length",
            controls.clone(),
        ));
    }
    let requested = usize::try_from(requested.min(available)).map_err(|_| {
        ProtocolError::new(
            "ENOSPC",
            "shared-memory size overflows this platform",
            controls.clone(),
        )
    })?;
    if requested > source_input_limit(controls)? {
        return Err(ProtocolError::new(
            "ENOSPC",
            "shared-memory source exceeds the decoder limit",
            controls.clone(),
        ));
    }
    if requested == 0 {
        return Ok(Vec::new());
    }

    // macOS POSIX shm descriptors cannot be read(2), but macOS does guarantee
    // that these objects cannot shrink while mapped, unlike ordinary files.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err(ProtocolError::new(
            "EIO",
            "could not determine the shared-memory page size",
            controls.clone(),
        ));
    }
    let page_size = page_size as u64;
    let map_offset = offset - offset % page_size;
    let delta = usize::try_from(offset - map_offset).map_err(|_| {
        ProtocolError::new("ENOSPC", "shared-memory offset overflows", controls.clone())
    })?;
    let map_length = requested.checked_add(delta).ok_or_else(|| {
        ProtocolError::new(
            "ENOSPC",
            "shared-memory mapping overflows",
            controls.clone(),
        )
    })?;
    let mapped = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            map_length,
            libc::PROT_READ,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            map_offset as libc::off_t,
        )
    };
    if mapped == libc::MAP_FAILED {
        return Err(io_error(std::io::Error::last_os_error(), controls));
    }
    let bytes =
        unsafe { std::slice::from_raw_parts((mapped as *const u8).add(delta), requested).to_vec() };
    unsafe {
        libc::munmap(mapped, map_length);
    }
    Ok(bytes)
}

#[cfg(windows)]
fn read_regular_file(
    name: &[u8],
    temporary: bool,
    controls: &Controls,
) -> Result<Vec<u8>, ProtocolError> {
    let name = std::str::from_utf8(name).map_err(|_| {
        ProtocolError::new(
            "EINVAL",
            "Windows graphics paths must be UTF-8",
            controls.clone(),
        )
    })?;
    if name.starts_with(r"\\.\") || name.starts_with(r"\\?\GLOBALROOT") {
        return Err(ProtocolError::new(
            "EACCES",
            "Windows device paths are not graphics files",
            controls.clone(),
        ));
    }
    let canonical = std::fs::canonicalize(name).map_err(|error| io_error(error, controls))?;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .open(&canonical)
        .map_err(|error| io_error(error, controls))?;
    if !file
        .metadata()
        .map_err(|error| io_error(error, controls))?
        .file_type()
        .is_file()
    {
        return Err(ProtocolError::new(
            "EBADF",
            "graphics source is not a regular file",
            controls.clone(),
        ));
    }
    let read_result = read_region(&mut file, controls);
    let cleanup_result = if temporary && safe_temporary_path(&canonical) {
        std::fs::remove_file(&canonical).map_err(|error| io_error(error, controls))
    } else {
        Ok(())
    };
    cleanup_result?;
    read_result
}

#[cfg(windows)]
fn safe_temporary_path(path: &std::path::Path) -> bool {
    let path = path.to_string_lossy();
    let temporary_root = std::env::temp_dir();
    path.contains("tty-graphics-protocol")
        && path.starts_with(temporary_root.to_string_lossy().as_ref())
}

#[cfg(windows)]
fn read_shared_memory(name: &[u8], controls: &Controls) -> Result<Vec<u8>, ProtocolError> {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Memory::{
        MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ,
        MEMORY_BASIC_INFORMATION,
    };

    let name = std::str::from_utf8(name).map_err(|_| {
        ProtocolError::new(
            "EINVAL",
            "Windows shared-memory names must be UTF-8",
            controls.clone(),
        )
    })?;
    let mut wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let maximum = source_input_limit(controls)?;
    let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, wide.as_mut_ptr()) };
    if handle.is_null() {
        return Err(io_error(std::io::Error::last_os_error(), controls));
    }
    // A zero length maps the whole object; we immediately query and bound the
    // view before copying. Windows has no public mapping-size query without a
    // view, and this preserves protocol support for an omitted S.
    let view = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0) };
    if view.Value.is_null() {
        unsafe { CloseHandle(handle) };
        return Err(io_error(std::io::Error::last_os_error(), controls));
    }
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { zeroed() };
    let queried =
        unsafe { VirtualQuery(view.Value, &mut info, size_of::<MEMORY_BASIC_INFORMATION>()) };
    let result = if queried == 0 {
        Err(io_error(std::io::Error::last_os_error(), controls))
    } else {
        let length = info.RegionSize;
        let offset = usize::try_from(controls.unsigned(b'O', 0)).unwrap_or(usize::MAX);
        let available = length.saturating_sub(offset);
        let compressed_png = controls.byte(b'o', 0) == b'z' && controls.unsigned(b'f', 32) == 100;
        let requested = if compressed_png {
            available
        } else {
            match controls.unsigned(b'S', 0) {
                0 if controls.byte(b'o', 0) == 0 && controls.unsigned(b'f', 32) != 100 => maximum,
                0 => available,
                size => size as usize,
            }
        };
        let processed = controls.byte(b'o', 0) == b'z' || controls.unsigned(b'f', 32) == 100;
        if offset > length || (!processed && requested > available) {
            Err(ProtocolError::new(
                "EINVAL",
                "shared-memory range exceeds its length",
                controls.clone(),
            ))
        } else {
            let requested = requested.min(available);
            if requested > maximum {
                Err(ProtocolError::new(
                    "ENOSPC",
                    "shared-memory source exceeds the decoder limit",
                    controls.clone(),
                ))
            } else {
                let source = unsafe {
                    std::slice::from_raw_parts(view.Value.cast::<u8>().add(offset), requested)
                };
                Ok(source.to_vec())
            }
        }
    };
    unsafe {
        UnmapViewOfFile(view);
        CloseHandle(handle);
    }
    result
}

#[cfg(not(any(unix, windows)))]
fn read_regular_file(
    _name: &[u8],
    _temporary: bool,
    controls: &Controls,
) -> Result<Vec<u8>, ProtocolError> {
    Err(ProtocolError::new(
        "ENOSYS",
        "file graphics transfers are unavailable on this platform",
        controls.clone(),
    ))
}

#[cfg(not(any(unix, windows)))]
fn read_shared_memory(_name: &[u8], controls: &Controls) -> Result<Vec<u8>, ProtocolError> {
    Err(ProtocolError::new(
        "ENOSYS",
        "shared-memory graphics transfers are unavailable on this platform",
        controls.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_raw_rgb_into_rgba() {
        let mut decoder = Decoder::default();
        let command = decoder
            .decode(b"Gf=24,s=1,v=1;AQID", true)
            .unwrap()
            .unwrap();
        let image = command.image.unwrap();
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(&*image.rgba, &[1, 2, 3, 255]);
    }

    #[test]
    fn chunks_are_atomic_and_propagate_final_q() {
        let mut decoder = Decoder::default();
        assert!(decoder
            .decode(b"Gf=24,s=2,v=1,m=1;AQID", true)
            .unwrap()
            .is_none());
        let command = decoder.decode(b"Gm=0,q=2;BAUG", true).unwrap().unwrap();
        assert_eq!(command.controls.byte(b'q', 0), 2);
        assert_eq!(&*command.image.unwrap().rgba, &[1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn malformed_headers_retain_response_controls() {
        let error = Controls::parse(b"f=99,i=7,p=9,q=2").unwrap_err();
        assert_eq!(error.controls.unsigned(b'i', 0), 7);
        assert_eq!(error.controls.unsigned(b'p', 0), 9);
        assert_eq!(error.controls.byte(b'q', 0), 2);
    }

    #[test]
    fn malformed_continuation_cancels_the_partial_upload() {
        let mut decoder = Decoder::default();
        assert!(decoder
            .decode(b"Gf=24,s=2,v=1,m=1;AQID", true)
            .unwrap()
            .is_none());
        assert!(decoder.decode(b"Gm=0,x=1;BAUG", true).is_err());
        assert!(decoder
            .decode(b"Gf=24,s=1,v=1;AQID", true)
            .unwrap()
            .is_some());
    }

    #[test]
    fn refuses_nul_and_oversized_raw_dimensions_before_display() {
        let mut decoder = Decoder::default();
        assert_eq!(
            decoder
                .decode(b"Gf=32,s=1,v=1;AAAA\0", true)
                .unwrap_err()
                .code,
            "EINVAL"
        );
        assert_eq!(
            decoder
                .decode(b"Gf=32,s=4096,v=4096;", true)
                .unwrap_err()
                .code,
            "ENOSPC"
        );
    }

    #[test]
    fn decodes_png_and_zlib_pixel_sources() {
        let mut decoder = Decoder::default();
        let mut png_source = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_source, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0, 255]).unwrap();
        }
        let png_command = format!(
            "Gf=100;{}",
            base64::engine::general_purpose::STANDARD.encode(&png_source),
        );
        let png = decoder
            .decode(png_command.as_bytes(), true)
            .unwrap()
            .unwrap()
            .image
            .unwrap();
        assert_eq!((png.width, png.height, png.rgba.len()), (1, 1, 4));
        assert_eq!(png.rgba.as_slice(), &[255, 0, 0, 255]);

        let mut compressed = Vec::new();
        {
            use std::io::Write as _;
            let mut encoder =
                flate2::write::ZlibEncoder::new(&mut compressed, flate2::Compression::default());
            encoder.write_all(&[1, 2, 3]).unwrap();
            encoder.finish().unwrap();
        }
        let payload = base64::engine::general_purpose::STANDARD.encode(compressed);
        let command = format!("Gf=24,s=1,v=1,o=z;{payload}");
        let image = decoder
            .decode(command.as_bytes(), true)
            .unwrap()
            .unwrap()
            .image
            .unwrap();
        assert_eq!(&*image.rgba, &[1, 2, 3, 255]);

        let mut compressed_png = Vec::new();
        {
            use std::io::Write as _;
            let mut encoder = flate2::write::ZlibEncoder::new(
                &mut compressed_png,
                flate2::Compression::default(),
            );
            encoder.write_all(&png_source).unwrap();
            encoder.finish().unwrap();
        }
        let payload = base64::engine::general_purpose::STANDARD.encode(compressed_png);
        let command = format!("Gf=100,o=z,S={};{payload}", png_source.len());
        let image = decoder
            .decode(command.as_bytes(), true)
            .unwrap()
            .unwrap()
            .image
            .unwrap();
        assert_eq!((image.width, image.height, image.rgba.len()), (1, 1, 4));
    }

    #[test]
    fn disallows_file_transfers_and_safely_removes_temporary_files() {
        let mut decoder = Decoder::default();
        let name = base64::engine::general_purpose::STANDARD.encode(b"/not-used");
        let command = format!("Gf=24,s=1,v=1,t=f;{name}");
        assert_eq!(
            decoder.decode(command.as_bytes(), false).unwrap_err().code,
            "EACCES"
        );

        #[cfg(unix)]
        {
            let path = std::env::temp_dir().join(format!(
                "tty-graphics-protocol-decoder-{}",
                std::process::id()
            ));
            std::fs::write(&path, [1, 2, 3]).unwrap();
            let encoded = base64::engine::general_purpose::STANDARD
                .encode(path.as_os_str().as_encoded_bytes());
            let command = format!("Gf=24,s=1,v=1,t=t;{encoded}");
            assert!(decoder.decode(command.as_bytes(), true).unwrap().is_some());
            assert!(!path.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn shared_memory_reads_unaligned_raw_data_without_size_and_unlinks_the_object() {
        use std::ffi::CString;
        use std::os::fd::FromRawFd as _;

        struct SharedName(CString);
        impl Drop for SharedName {
            fn drop(&mut self) {
                unsafe {
                    libc::shm_unlink(self.0.as_ptr());
                }
            }
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let name =
            SharedName(CString::new(format!("/yttt-{}-{nonce}", std::process::id())).unwrap());
        let fd = unsafe {
            libc::shm_open(
                name.0.as_ptr(),
                libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
                0o600 as libc::c_uint,
            )
        };
        assert!(fd >= 0, "{}", std::io::Error::last_os_error());
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        assert_eq!(unsafe { libc::ftruncate(fd, 8) }, 0);
        let mapped = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                8,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        assert_ne!(mapped, libc::MAP_FAILED);
        unsafe {
            std::ptr::copy_nonoverlapping([1u8, 2, 3, 4, 5, 6, 7, 8].as_ptr(), mapped.cast(), 8);
            libc::munmap(mapped, 8);
        }
        drop(file);
        let encoded = base64::engine::general_purpose::STANDARD.encode(name.0.as_bytes());
        let command = format!("Ga=q,i=20,t=s,f=32,s=1,v=1,O=4;{encoded}");
        let image = Decoder::default()
            .decode(command.as_bytes(), true)
            .unwrap()
            .unwrap()
            .image
            .unwrap();
        assert_eq!(image.rgba.as_slice(), &[5, 6, 7, 8]);
        assert_eq!(
            unsafe { libc::shm_open(name.0.as_ptr(), libc::O_RDONLY, 0) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ENOENT)
        );
    }
}
