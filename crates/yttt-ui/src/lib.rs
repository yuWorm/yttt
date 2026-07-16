pub mod primitives;
pub mod style;
pub mod theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableState {
    Active,
    Inactive,
}
