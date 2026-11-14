//! How a project's layout is chosen when it opens, shared by the desktop and
//! the mobile Clients: the project's own layout file, else the profile's
//! default template; then the user's personal layout over it. Pure: callers
//! read the files, through whatever storage they have.

pub mod compose;
pub mod overrides;
pub mod personal;
pub mod project;
pub mod recent;
pub mod template;
