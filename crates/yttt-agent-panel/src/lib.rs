//! The agent panel: a composer with `@` mentions and `/` commands, and a
//! thread of streaming messages, thinking blocks and tool call cards. The
//! panel renders [`event::ThreadEvent`]s and reports what the user asked for;
//! whatever runs the agent sits outside it.
pub mod composer;
pub mod demo;
pub mod diff;
pub mod event;
pub mod model;
pub mod models;
pub mod panel;
pub mod slots;
pub mod text;
pub mod thread_view;

pub use composer::{Candidate, CandidateSource, SelectorKind, SelectorOption};
pub use panel::{AgentPanel, AgentPanelEvent};
pub use slots::{Slot, SlotContext, SlotItem, StatusItem};
