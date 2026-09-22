use std::{
    io::Cursor,
    path::Path,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui::{
    App, Bounds, Context, EventEmitter, FocusHandle, Focusable, MouseButton, Pixels, Point,
    RenderImage, Window, canvas, div, img, prelude::*, px,
};
use gpui_component::{
    Sizable,
    menu::{ContextMenuExt, PopupMenuItem},
};
use image::{AnimationDecoder, ImageDecoder};

use crate::ui::{
    i18n::{UiText, UiTextKey},
    theme::{current_ui_style, current_workbench_theme},
};
use yttt_ui::primitives::button::{YtttButtonVariant, yttt_button};

pub const MAX_PREVIEW_BYTES: u64 = 32 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 16384;
const MAX_ANIMATION_FRAMES: usize = 256;

pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            gpui::Img::extensions()
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
}

pub fn is_svg_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
}

pub struct DecodedPreview {
    pub image: Arc<RenderImage>,
    pub width: u32,
    pub height: u32,
    pub byte_len: usize,
    pub format: String,
}

fn checked_dimensions(width: u32, height: u32) -> anyhow::Result<u64> {
    let bytes = u64::from(width) * u64::from(height) * 4;
    anyhow::ensure!(
        width > 0
            && height > 0
            && width <= MAX_IMAGE_DIMENSION
            && height <= MAX_IMAGE_DIMENSION
            && bytes <= MAX_DECODED_BYTES,
        "Image exceeds the preview pixel limit"
    );
    Ok(bytes)
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    limits
}

fn animation_frames(
    decoder: impl for<'a> AnimationDecoder<'a>,
) -> anyhow::Result<Vec<image::Frame>> {
    let mut frames = Vec::new();
    let mut decoded_bytes = 0;
    for frame in decoder.into_frames() {
        anyhow::ensure!(
            frames.len() < MAX_ANIMATION_FRAMES,
            "Animation exceeds the preview frame limit"
        );
        let frame = frame?;
        decoded_bytes += checked_dimensions(frame.buffer().width(), frame.buffer().height())?;
        anyhow::ensure!(
            decoded_bytes <= MAX_DECODED_BYTES,
            "Animation exceeds the preview memory limit"
        );
        frames.push(frame);
    }
    anyhow::ensure!(!frames.is_empty(), "Image has no frames");
    Ok(frames)
}

/// Decode off the UI thread. Never hand unbounded compressed data to GPUI's cache.
pub fn decode_preview(bytes: Vec<u8>, svg: bool) -> anyhow::Result<DecodedPreview> {
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_PREVIEW_BYTES,
        "File exceeds the preview size limit"
    );
    let byte_len = bytes.len();
    let (mut frames, format) = if svg {
        let rejected_reference = Arc::new(AtomicBool::new(false));
        let data_rejected = rejected_reference.clone();
        let string_rejected = rejected_reference.clone();
        static FONTS: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
            let mut fonts = resvg::usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        });
        let options = resvg::usvg::Options {
            // Never resolve a remote SVG's references against the client's filesystem.
            image_href_resolver: resvg::usvg::ImageHrefResolver {
                resolve_data: Box::new(move |_, _, _| {
                    data_rejected.store(true, Ordering::Relaxed);
                    None
                }),
                resolve_string: Box::new(move |_, _| {
                    string_rejected.store(true, Ordering::Relaxed);
                    None
                }),
            },
            fontdb: FONTS.clone(),
            ..Default::default()
        };
        let tree = resvg::usvg::Tree::from_data(&bytes, &options)?;
        anyhow::ensure!(
            !rejected_reference.load(Ordering::Relaxed),
            "SVG images with embedded or external image references must be opened externally"
        );
        let dimensions = tree.size().to_int_size();
        checked_dimensions(dimensions.width(), dimensions.height())?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(dimensions.width(), dimensions.height())
            .ok_or_else(|| anyhow::anyhow!("Invalid SVG dimensions"))?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::identity(),
            &mut pixmap.as_mut(),
        );
        let mut rgba = pixmap.take();
        // tiny-skia pixels are premultiplied; GPUI expects straight-alpha BGRA.
        for pixel in rgba.chunks_exact_mut(4) {
            if pixel[3] != 0 {
                for channel in 0..3 {
                    pixel[channel] =
                        (u32::from(pixel[channel]) * 255 / u32::from(pixel[3])).min(255) as u8;
                }
            }
        }
        let buffer = image::RgbaImage::from_raw(dimensions.width(), dimensions.height(), rgba)
            .ok_or_else(|| anyhow::anyhow!("Invalid SVG pixels"))?;
        (vec![image::Frame::new(buffer)], "SVG".to_string())
    } else {
        let format = image::guess_format(&bytes)?;
        let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
        reader.limits(limits());
        let dimensions = reader.into_dimensions()?;
        checked_dimensions(dimensions.0, dimensions.1)?;
        let frames = match format {
            image::ImageFormat::Gif => {
                let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes))?;
                decoder.set_limits(limits())?;
                animation_frames(decoder)?
            }
            image::ImageFormat::WebP => {
                let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))?;
                decoder.set_limits(limits())?;
                if decoder.has_animation() {
                    animation_frames(decoder)?
                } else {
                    vec![image::Frame::new(
                        image::DynamicImage::from_decoder(decoder)?.into_rgba8(),
                    )]
                }
            }
            _ => {
                let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
                reader.limits(limits());
                vec![image::Frame::new(reader.decode()?.into_rgba8())]
            }
        };
        (frames, format!("{format:?}"))
    };
    let width = frames[0].buffer().width();
    let height = frames[0].buffer().height();
    for frame in &mut frames {
        for pixel in frame.buffer_mut().chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    Ok(DecodedPreview {
        image: Arc::new(RenderImage::new(frames)),
        width,
        height,
        byte_len,
        format,
    })
}

#[derive(Clone, Copy)]
pub enum FilePreviewEvent {
    OpenExternal,
    ShowSource,
    Reload,
}

pub struct FilePreview {
    focus: FocusHandle,
    pub relative_path: std::path::PathBuf,
    decoded: Option<DecodedPreview>,
    error: Option<String>,
    loading: bool,
    text: UiText,
    download: bool,
    zoom: Option<f32>,
    pan: Point<Pixels>,
    drag: Option<Point<Pixels>>,
    bounds: Bounds<Pixels>,
}

impl EventEmitter<FilePreviewEvent> for FilePreview {}
impl Focusable for FilePreview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FilePreview {
    pub fn new(
        relative_path: std::path::PathBuf,
        text: UiText,
        download: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|this, cx| {
            if let Some(decoded) = this.decoded.take() {
                cx.drop_image(decoded.image, None);
            }
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            relative_path,
            decoded: None,
            error: None,
            loading: true,
            text,
            download,
            zoom: None,
            pan: Point::default(),
            drag: None,
            bounds: Bounds::default(),
        }
    }

    pub fn complete(&mut self, result: Result<DecodedPreview, String>, cx: &mut Context<Self>) {
        if let Some(previous) = self.decoded.take() {
            cx.drop_image(previous.image, None);
        }
        self.loading = false;
        self.error = None;
        match result {
            Ok(decoded) => self.decoded = Some(decoded),
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    pub fn start_loading(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        cx.notify();
    }

    pub fn set_text(&mut self, text: UiText, download: bool) {
        self.text = text;
        self.download = download;
    }

    fn scale(&self) -> f32 {
        self.zoom.unwrap_or_else(|| {
            self.decoded.as_ref().map_or(1.0, |image| {
                (f32::from(self.bounds.size.width) / image.width as f32)
                    .min(f32::from(self.bounds.size.height) / image.height as f32)
                    .clamp(0.001, 1.0)
            })
        })
    }

    fn change_zoom(&mut self, factor: f32, cx: &mut Context<Self>) {
        self.zoom = Some((self.scale() * factor).clamp(0.01, 32.0));
        cx.notify();
    }
}

impl Render for FilePreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = current_workbench_theme(cx);
        let style = current_ui_style(cx);
        let button = |id: &'static str, label: &'static str| {
            yttt_button(id, label, YtttButtonVariant::Secondary, theme, style, cx)
                .small()
                .debug_selector(move || format!("file-preview-{id}"))
        };
        let external_text = self.text.get(if self.download {
            UiTextKey::FileDownloadOpen
        } else {
            UiTextKey::FileOpenExternal
        });
        let external_view = cx.weak_entity();
        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .p_2()
            .child(
                button("fit", self.text.get(UiTextKey::FilePreviewFit)).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.zoom = None;
                        this.pan = Point::default();
                        cx.notify();
                    },
                )),
            )
            .child(
                button("actual", self.text.get(UiTextKey::FilePreviewActualSize)).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.zoom = Some(1.0);
                        this.pan = Point::default();
                        cx.notify();
                    }),
                ),
            )
            .child(
                button("zoom-out", "−")
                    .on_click(cx.listener(|this, _, _, cx| this.change_zoom(1.0 / 1.2, cx))),
            )
            .child(
                button("zoom-in", "+")
                    .on_click(cx.listener(|this, _, _, cx| this.change_zoom(1.2, cx))),
            )
            .child(
                button("reload", self.text.get(UiTextKey::FilePreviewReload))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(FilePreviewEvent::Reload))),
            )
            .when(is_svg_path(&self.relative_path), |bar| {
                bar.child(
                    button("source", self.text.get(UiTextKey::FilePreviewSource))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(FilePreviewEvent::ShowSource))),
                )
            })
            .child(div().flex_1())
            .child(
                button("external", external_text)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(FilePreviewEvent::OpenExternal))),
            );
        let mut viewport = div()
            .id("file-preview-viewport")
            .debug_selector(|| "file-preview-viewport".into())
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden();
        if self.decoded.is_some() {
            let measure_view = cx.weak_entity();
            viewport = viewport.child(
                canvas(
                    move |bounds, window, cx| {
                        // Fit using this frame's bounds, not the previous render's measurements.
                        let geometry = measure_view
                            .update(cx, |this, _| {
                                let changed = this.bounds != bounds;
                                this.bounds = bounds;
                                let decoded = this.decoded.as_ref()?;
                                Some((
                                    decoded.image.clone(),
                                    px(decoded.width as f32 * this.scale()),
                                    px(decoded.height as f32 * this.scale()),
                                    this.pan,
                                    changed,
                                ))
                            })
                            .ok()
                            .flatten();
                        let (image, width, height, pan, changed) = geometry?;
                        if changed {
                            let view = measure_view.clone();
                            window.defer(cx, move |_, cx| {
                                let _ = view.update(cx, |_, cx| cx.notify());
                            });
                        }
                        let mut content = div()
                            .relative()
                            .size_full()
                            .child(
                                img(image)
                                    .id("preview-image")
                                    .debug_selector(|| "preview-image".into())
                                    .absolute()
                                    .w(width)
                                    .h(height)
                                    .left((bounds.size.width - width) / 2.0 + pan.x)
                                    .top((bounds.size.height - height) / 2.0 + pan.y),
                            )
                            .into_any_element();
                        content.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
                        Some(content)
                    },
                    |bounds, content, window, cx| {
                        window.paint_quad(gpui::fill(
                            bounds,
                            gpui::checkerboard(gpui::rgb(0x303030), 16.0),
                        ));
                        if let Some(mut content) = content {
                            content.paint(window, cx);
                        }
                    },
                )
                .absolute()
                .size_full(),
            );
        } else {
            viewport = viewport
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .p_4()
                .child(self.text.get(if self.loading {
                    UiTextKey::FilePreviewLoading
                } else {
                    UiTextKey::FilePreviewUnavailable
                }))
                .children(self.error.clone());
        }
        let metadata = self.decoded.as_ref().map(|image| {
            format!(
                "{} × {} · {} · {} B · {:.0}%",
                image.width,
                image.height,
                image.format,
                image.byte_len,
                self.scale() * 100.0
            )
        });
        div()
            .id("file-preview")
            .track_focus(&self.focus)
            .key_context("FilePreview")
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .text_color(theme.text)
            .child(toolbar)
            .child(
                viewport
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.drag = Some(event.position);
                            cx.notify();
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some(previous) = this.drag {
                            if event.pressed_button != Some(MouseButton::Left) {
                                this.drag = None;
                                return;
                            }
                            this.pan += event.position - previous;
                            this.drag = Some(event.position);
                            cx.notify();
                        }
                    }))
                    .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                        this.change_zoom(
                            (f32::from(event.delta.pixel_delta(px(20.0)).y) * 0.005).exp(),
                            cx,
                        );
                        cx.stop_propagation();
                    }))
                    .on_pinch(cx.listener(|this, event: &gpui::PinchEvent, _, cx| {
                        this.change_zoom(1.0 + event.delta, cx)
                    })),
            )
            .child(
                div()
                    .p_2()
                    .text_sm()
                    .children(metadata)
                    .when(self.download, |footer| {
                        footer.child(self.text.get(UiTextKey::FileDownloadedCopy))
                    }),
            )
            .context_menu(move |menu, _, cx| {
                let view = external_view.clone();
                yttt_ui::primitives::menu::yttt_popup_menu(
                    menu,
                    current_workbench_theme(cx),
                    current_ui_style(cx),
                )
                .item(PopupMenuItem::new(external_text).on_click(
                    move |_, _, cx| {
                        let _ = view.update(cx, |_, cx| cx.emit(FilePreviewEvent::OpenExternal));
                    },
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding_preserves_pixels_and_animation_frames() {
        let pixel = image::RgbaImage::from_pixel(2, 3, image::Rgba([240, 30, 10, 255]));
        let mut png = Cursor::new(Vec::new());
        pixel.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let decoded = decode_preview(png.into_inner(), false).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 3));
        assert_eq!(
            decoded.image.as_bytes(0).unwrap(),
            [10, 30, 240, 255].repeat(6)
        );
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            encoder
                .encode_frame(image::Frame::new(pixel.clone()))
                .unwrap();
            encoder.encode_frame(image::Frame::new(pixel)).unwrap();
        }
        let decoded = decode_preview(gif, false).unwrap();
        assert_eq!(decoded.image.frame_count(), 2);
    }

    #[gpui::test]
    fn image_is_sized_on_the_first_layout(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let image = image::RgbaImage::new(120, 80);
        let decoded = DecodedPreview {
            image: Arc::new(RenderImage::new(vec![image::Frame::new(image)])),
            width: 120,
            height: 80,
            byte_len: 0,
            format: "PNG".into(),
        };
        let (_, cx) = cx.add_window_view(|_, cx| {
            let mut view = FilePreview::new("sample.png".into(), UiText::english(), false, cx);
            view.complete(Ok(decoded), cx);
            view
        });
        cx.refresh().unwrap();
        let image_bounds = cx.debug_bounds("preview-image").unwrap();
        assert_eq!(image_bounds.size, gpui::size(px(120.0), px(80.0)));
    }

    #[test]
    fn svg_rejects_oversized_rasters_and_external_references() {
        assert!(
            decode_preview(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="20000" height="20000"/>"#
                    .to_vec(),
                true
            )
            .is_err()
        );
        assert!(decode_preview(br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="file:///etc/passwd" width="10" height="10"/></svg>"#.to_vec(), true).is_err());
        let decoded = decode_preview(br#"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="8"><rect width="12" height="8" fill="red"/></svg>"#.to_vec(), true).unwrap();
        assert_eq!((decoded.width, decoded.height), (12, 8));
    }

    #[test]
    fn corrupt_images_and_excessive_animations_are_rejected() {
        assert!(decode_preview(b"not an image".to_vec(), false).is_err());
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            for _ in 0..=MAX_ANIMATION_FRAMES {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::new(1, 1)))
                    .unwrap();
            }
        }
        assert!(decode_preview(gif, false).is_err());
    }
}
