use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PostcardV1 {
    a: u32,
    b: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PostcardV1PlusField {
    a: u32,
    b: u32,
    #[serde(default)]
    c: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct PostcardV1MinusField {
    a: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum PostcardOldEnum {
    A,
    B,
    C,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum PostcardNewEnum {
    A,
    Inserted,
    B,
    C,
}

#[test]
fn postcard_cannot_decode_an_older_struct_after_a_field_is_appended() {
    let encoded = postcard::to_allocvec(&PostcardV1 { a: 1, b: 2 }).unwrap();
    assert!(
        postcard::from_bytes::<PostcardV1PlusField>(&encoded).is_err(),
        "postcard has no field tags, so serde(default) cannot recover a newly appended field"
    );
}

#[test]
fn postcard_leaves_unread_bytes_or_fails_when_a_field_is_removed() {
    let encoded = postcard::to_allocvec(&PostcardV1 { a: 1, b: 2 }).unwrap();
    let decoded = postcard::from_bytes::<PostcardV1MinusField>(&encoded);
    assert!(
        decoded.is_err() || decoded == Ok(PostcardV1MinusField { a: 1 }),
        "removing a postcard field either fails or silently drops trailing bytes"
    );
}

#[test]
fn postcard_reinterprets_later_enum_variants_when_a_variant_is_inserted() {
    let encoded = postcard::to_allocvec(&PostcardOldEnum::B).unwrap();
    let decoded: PostcardNewEnum = postcard::from_bytes(&encoded).unwrap();
    assert_eq!(
        decoded,
        PostcardNewEnum::Inserted,
        "postcard numbers enums by declaration order, so inserting a variant silently shifts later ones"
    );
}
