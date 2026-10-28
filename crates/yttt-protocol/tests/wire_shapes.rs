//! Freezes the variant and field names of every resource and lifecycle wire type.
//!
//! CBOR carries enum variants and struct fields by name, so the names are the
//! compatibility contract with peers built from other versions. Removing or renaming one
//! breaks those peers and requires a protocol version bump. Appending one is compatible,
//! provided a new struct field carries `#[serde(default)]`; re-bless the fixture with
//! `YTTT_BLESS_WIRE_SHAPES=1 cargo test -p yttt-protocol --test wire_shapes`.
//!
//! Names are read from the `fields`/`variants` lists serde derive passes to the
//! deserializer. A probe deserializer walks every root type repeatedly, entering the
//! next variant of each enum on each pass, until every recorded variant and field has
//! been entered. Self-describing subtrees (`serde_json::Value`, internally tagged,
//! untagged, and flattened types) only reach `deserialize_any` and stay opaque.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    path::PathBuf,
};

use serde::{
    Deserialize,
    de::{
        self, DeserializeOwned, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess,
        SeqAccess, VariantAccess, Visitor,
    },
};
use yttt_protocol::{
    ControlMessage, HandshakeMessage, HostEvent, LifecycleMessage, TerminalInteractiveMessage,
};

const FIXTURE: &str = "tests/fixtures/wire-shapes-v22.json";
/// Containers deeper than this get empty sequences, maps, and `None`.
const EXPAND_DEPTH: usize = 24;
/// Recursive enums without a terminating variant abort the walk here.
const ABORT_DEPTH: usize = 64;
const MAX_PASSES: usize = 200_000;

/// Container name → member → struct field names. Struct fields use the member `""`.
type Shapes = BTreeMap<String, BTreeMap<String, BTreeSet<String>>>;

#[derive(Default)]
struct Walk {
    shapes: Shapes,
    /// Every (container key, member) that must be entered at least once.
    required: BTreeSet<(String, String)>,
    entered: BTreeSet<(String, String)>,
    next_variant: HashMap<&'static str, usize>,
    field_failures: HashMap<(String, &'static str), u32>,
}

impl Walk {
    fn record_fields(&mut self, container: &str, member: &str, fields: &[&'static str]) {
        let key = if member.is_empty() {
            container.to_string()
        } else {
            format!("{container}::{member}")
        };
        self.shapes
            .entry(container.to_string())
            .or_default()
            .entry(member.to_string())
            .or_default()
            .extend(fields.iter().map(|field| field.to_string()));
        self.required
            .extend(fields.iter().map(|field| (key.clone(), field.to_string())));
    }

    /// Orders fields so that the ones that failed least often are entered first.
    fn field_order(&self, key: &str, fields: &[&'static str]) -> Vec<&'static str> {
        let mut ordered = fields.to_vec();
        ordered.sort_by_key(|field| {
            self.field_failures
                .get(&(key.to_string(), *field))
                .copied()
                .unwrap_or(0)
        });
        ordered
    }
}

#[derive(Debug)]
struct ProbeError(String);

impl fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProbeError {}

impl de::Error for ProbeError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

type Result<T, E = ProbeError> = std::result::Result<T, E>;

#[derive(Clone, Copy)]
struct Probe<'a> {
    walk: &'a RefCell<Walk>,
    depth: usize,
}

impl<'a> Probe<'a> {
    fn deeper(self) -> Result<Self> {
        if self.depth >= ABORT_DEPTH {
            return Err(ProbeError("recursion limit".to_string()));
        }
        Ok(Self {
            walk: self.walk,
            depth: self.depth + 1,
        })
    }

    fn expands(self) -> bool {
        self.depth < EXPAND_DEPTH
    }

    fn fields<'de, V: Visitor<'de>>(
        self,
        key: String,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        let order = self.walk.borrow().field_order(&key, fields);
        visitor.visit_map(Fields {
            probe: self.deeper()?,
            key,
            order: order.into_iter(),
            current: None,
        })
    }
}

macro_rules! scalar {
    ($($method:ident => $visit:ident($value:expr)),+ $(,)?) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
            visitor.$visit($value)
        }
    )+};
}

impl<'de, 'a> de::Deserializer<'de> for Probe<'a> {
    type Error = ProbeError;

    scalar!(
        deserialize_bool => visit_bool(false),
        deserialize_i8 => visit_i64(1),
        deserialize_i16 => visit_i64(1),
        deserialize_i32 => visit_i64(1),
        deserialize_i64 => visit_i64(1),
        deserialize_i128 => visit_i64(1),
        deserialize_u8 => visit_u64(1),
        deserialize_u16 => visit_u64(1),
        deserialize_u32 => visit_u64(1),
        deserialize_u64 => visit_u64(1),
        deserialize_u128 => visit_u64(1),
        deserialize_f32 => visit_f64(1.0),
        deserialize_f64 => visit_f64(1.0),
        deserialize_char => visit_char('a'),
        deserialize_str => visit_str("a"),
        deserialize_string => visit_str("a"),
        deserialize_identifier => visit_str("a"),
        deserialize_bytes => visit_bytes(b"a"),
        deserialize_byte_buf => visit_byte_buf(b"a".to_vec()),
    );

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_unit()
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_unit()
    }

    /// Self-describing content: an empty map satisfies `serde_json::Value` objects and
    /// leaves tagged, untagged, and flattened types opaque.
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_map(Entries {
            probe: self,
            remaining: 0,
        })
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        if self.expands() {
            visitor.visit_some(self.deeper()?)
        } else {
            visitor.visit_none()
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_newtype_struct(self.deeper()?)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let remaining = usize::from(self.expands());
        visitor.visit_seq(Elements {
            probe: self.deeper()?,
            remaining,
        })
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value> {
        visitor.visit_seq(Elements {
            probe: self.deeper()?,
            remaining: len,
        })
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value> {
        self.deserialize_tuple(len, visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let remaining = usize::from(self.expands());
        visitor.visit_map(Entries {
            probe: self.deeper()?,
            remaining,
        })
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        self.walk.borrow_mut().record_fields(name, "", fields);
        self.fields(name.to_string(), fields, visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        let variant = {
            let mut walk = self.walk.borrow_mut();
            let members = walk.shapes.entry(name.to_string()).or_default();
            for variant in variants {
                members.entry(variant.to_string()).or_default();
            }
            walk.required.extend(
                variants
                    .iter()
                    .map(|variant| (name.to_string(), variant.to_string())),
            );
            let cursor = walk.next_variant.entry(name).or_default();
            let variant = variants[*cursor % variants.len()];
            *cursor += 1;
            walk.entered.insert((name.to_string(), variant.to_string()));
            variant
        };
        visitor.visit_enum(Variant {
            probe: self.deeper()?,
            name,
            variant,
        })
    }
}

struct Elements<'a> {
    probe: Probe<'a>,
    remaining: usize,
}

impl<'de, 'a> SeqAccess<'de> for Elements<'a> {
    type Error = ProbeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        seed.deserialize(self.probe).map(Some)
    }
}

struct Entries<'a> {
    probe: Probe<'a>,
    remaining: usize,
}

impl<'de, 'a> MapAccess<'de> for Entries<'a> {
    type Error = ProbeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        seed.deserialize(self.probe).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        seed.deserialize(self.probe)
    }
}

struct Fields<'a> {
    probe: Probe<'a>,
    key: String,
    order: std::vec::IntoIter<&'static str>,
    current: Option<&'static str>,
}

impl<'de, 'a> MapAccess<'de> for Fields<'a> {
    type Error = ProbeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        let Some(field) = self.order.next() else {
            return Ok(None);
        };
        self.current = Some(field);
        seed.deserialize(IntoDeserializer::<ProbeError>::into_deserializer(field))
            .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        let field = self.current.expect("value follows its key");
        self.probe
            .walk
            .borrow_mut()
            .entered
            .insert((self.key.clone(), field.to_string()));
        seed.deserialize(self.probe).inspect_err(|_| {
            *self
                .probe
                .walk
                .borrow_mut()
                .field_failures
                .entry((self.key.clone(), field))
                .or_default() += 1;
        })
    }
}

struct Variant<'a> {
    probe: Probe<'a>,
    name: &'static str,
    variant: &'static str,
}

impl<'de, 'a> EnumAccess<'de> for Variant<'a> {
    type Error = ProbeError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self)> {
        let value = seed.deserialize(IntoDeserializer::<ProbeError>::into_deserializer(
            self.variant,
        ))?;
        Ok((value, self))
    }
}

impl<'de, 'a> VariantAccess<'de> for Variant<'a> {
    type Error = ProbeError;

    fn unit_variant(self) -> Result<()> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value> {
        seed.deserialize(self.probe)
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value> {
        de::Deserializer::deserialize_tuple(self.probe, len, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        self.probe
            .walk
            .borrow_mut()
            .record_fields(self.name, self.variant, fields);
        self.probe
            .fields(format!("{}::{}", self.name, self.variant), fields, visitor)
    }
}

fn walk_root<T: DeserializeOwned>(walk: &RefCell<Walk>) {
    // Failures only end this pass early; names recorded before them are kept.
    let _ = T::deserialize(Probe { walk, depth: 0 });
}

fn trace() -> Shapes {
    let walk = RefCell::new(Walk::default());
    for _ in 0..MAX_PASSES {
        // Wire roots: handshake, lifecycle, control, terminal-interactive, and state-event frames.
        walk_root::<HandshakeMessage>(&walk);
        walk_root::<LifecycleMessage>(&walk);
        walk_root::<ControlMessage>(&walk);
        walk_root::<TerminalInteractiveMessage>(&walk);
        walk_root::<HostEvent>(&walk);
        let walk = walk.borrow();
        if walk.required.is_subset(&walk.entered) {
            return walk.shapes.clone();
        }
    }
    let walk = walk.into_inner();
    let missing: Vec<_> = walk.required.difference(&walk.entered).collect();
    panic!("wire walk did not reach: {missing:?}");
}

fn describe(container: &str, member: &str) -> String {
    if member.is_empty() {
        format!("{container} field")
    } else {
        format!("{container}::{member}")
    }
}

#[test]
fn wire_variant_and_field_names_are_append_only() {
    let current = trace();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    if std::env::var_os("YTTT_BLESS_WIRE_SHAPES").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&current).unwrap() + "\n",
        )
        .unwrap();
        return;
    }
    let frozen: Shapes = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{FIXTURE}: {error}")),
    )
    .unwrap();

    let mut removed = Vec::new();
    let mut added = Vec::new();
    for (container, members) in &frozen {
        let Some(now) = current.get(container) else {
            removed.push(format!("type {container}"));
            continue;
        };
        for (member, fields) in members {
            let Some(now_fields) = now.get(member) else {
                removed.push(format!("variant {}", describe(container, member)));
                continue;
            };
            for field in fields.difference(now_fields) {
                removed.push(format!("{} `{field}`", describe(container, member)));
            }
            for field in now_fields.difference(fields) {
                added.push(format!("{} `{field}`", describe(container, member)));
            }
        }
        for member in now.keys().filter(|member| !members.contains_key(*member)) {
            added.push(format!("variant {}", describe(container, member)));
        }
    }
    for container in current
        .keys()
        .filter(|container| !frozen.contains_key(*container))
    {
        added.push(format!("type {container}"));
    }

    assert!(
        removed.is_empty(),
        "removed or renamed wire names break peers built from other versions:\n  {}\n\
         Restore them, or bump RESOURCE_PROTOCOL_VERSION / LIFECYCLE_PROTOCOL_VERSION and \
         keep the Host serving the previous version. A Rust-only type rename is safe: re-bless.",
        removed.join("\n  "),
    );
    assert!(
        added.is_empty(),
        "new wire names are compatible appends:\n  {}\n\
         New struct fields need #[serde(default)]. Re-bless with \
         YTTT_BLESS_WIRE_SHAPES=1 cargo test -p yttt-protocol --test wire_shapes",
        added.join("\n  "),
    );
}

#[test]
fn probe_records_nested_variants_and_fields() {
    #[derive(Deserialize)]
    #[allow(dead_code)]
    enum Outer {
        Plain,
        Wrapped(Inner),
        Shaped { first: u8, inner: Option<Inner> },
    }
    #[derive(Deserialize)]
    #[allow(dead_code)]
    enum Inner {
        Left { value: String },
        Right,
    }
    let walk = RefCell::new(Walk::default());
    for _ in 0..8 {
        walk_root::<Outer>(&walk);
    }
    let walk = walk.into_inner();
    assert!(walk.required.is_subset(&walk.entered));
    let names = |container: &str| -> Vec<(String, Vec<String>)> {
        walk.shapes[container]
            .iter()
            .map(|(member, fields)| (member.clone(), fields.iter().cloned().collect()))
            .collect()
    };
    assert_eq!(
        names("Outer"),
        [
            ("Plain".to_string(), vec![]),
            (
                "Shaped".to_string(),
                vec!["first".to_string(), "inner".to_string()]
            ),
            ("Wrapped".to_string(), vec![]),
        ]
    );
    assert_eq!(
        names("Inner"),
        [
            ("Left".to_string(), vec!["value".to_string()]),
            ("Right".to_string(), vec![]),
        ]
    );
}
