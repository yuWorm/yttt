pub mod menu;
pub mod primitives;
pub mod style;
pub mod theme;
pub mod window;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableState {
    Active,
    Inactive,
}
