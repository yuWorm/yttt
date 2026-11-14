//! A project's layout with the user's personal layout over it.

use yttt_core::model::layout::ProjectLayout;

use crate::{
    overrides::{MergeWarning, merge_layouts},
    personal::{self, PersonalLayout, PersonalLayoutFileError},
};

/// The personal layout file as read.
#[derive(Clone, Copy, Debug)]
pub enum PersonalSource<'a> {
    Missing,
    /// It exists but could not be read; the message says why.
    Unreadable(&'a str),
    Text(&'a str),
}

/// What the personal layout did to the base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Personal {
    /// None applied: missing, or unusable (see the warnings).
    None,
    Patch,
    Replace,
}

/// Why a personal layout, or part of it, was left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PersonalWarning {
    Read(String),
    Parse(String),
    Validation(String),
    StaleTab(String),
    StalePane(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WithPersonal {
    pub layout: ProjectLayout,
    pub personal: Personal,
    pub warnings: Vec<PersonalWarning>,
}

/// Applies the personal layout to `base`: a replacement takes its place, a
/// patch merges into it. An unusable personal layout leaves `base` as it is,
/// with a warning.
pub fn apply_personal(base: ProjectLayout, source: PersonalSource<'_>) -> WithPersonal {
    let unchanged = |base, warning| WithPersonal {
        layout: base,
        personal: Personal::None,
        warnings: vec![warning],
    };
    let text = match source {
        PersonalSource::Missing => {
            return WithPersonal {
                layout: base,
                personal: Personal::None,
                warnings: Vec::new(),
            };
        }
        PersonalSource::Unreadable(message) => {
            return unchanged(base, PersonalWarning::Read(message.to_string()));
        }
        PersonalSource::Text(text) => text,
    };
    match personal::parse(text) {
        Err(PersonalLayoutFileError::Parse(message)) => {
            unchanged(base, PersonalWarning::Parse(message))
        }
        Err(PersonalLayoutFileError::Validation(message)) => {
            unchanged(base, PersonalWarning::Validation(message))
        }
        Ok(PersonalLayout::Replace(layout)) => WithPersonal {
            layout,
            personal: Personal::Replace,
            warnings: Vec::new(),
        },
        Ok(PersonalLayout::Patch(patch)) => match merge_layouts(&base, &patch) {
            Ok(merged) => WithPersonal {
                layout: merged.layout,
                personal: Personal::Patch,
                warnings: merged
                    .warnings
                    .into_iter()
                    .map(|warning| match warning {
                        MergeWarning::StaleTabOverride(id) => PersonalWarning::StaleTab(id),
                        MergeWarning::StalePaneOverride(id) => PersonalWarning::StalePane(id),
                    })
                    .collect(),
            },
            Err(error) => unchanged(base, PersonalWarning::Validation(error.to_string())),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::DefaultLayoutTemplate;

    fn base() -> ProjectLayout {
        DefaultLayoutTemplate::builtin().materialize("Demo")
    }

    #[test]
    fn a_missing_personal_layout_keeps_the_base() {
        let result = apply_personal(base(), PersonalSource::Missing);
        assert_eq!(result.layout, base());
        assert_eq!(result.personal, Personal::None);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn a_patch_renames_and_reports_stale_tabs() {
        let patch = r#"
            version = 1
            mode = "patch"
            [layout.project]
            name = "Renamed"
            [[layout.tabs]]
            id = "gone"
            title = "Gone"
        "#;
        let result = apply_personal(base(), PersonalSource::Text(patch));
        assert_eq!(result.personal, Personal::Patch);
        assert_eq!(result.layout.project.name, "Renamed");
        assert_eq!(
            result.warnings,
            vec![PersonalWarning::StaleTab("gone".into())]
        );
    }

    #[test]
    fn an_unparsable_personal_layout_keeps_the_base_with_a_warning() {
        let result = apply_personal(base(), PersonalSource::Text("not = [toml"));
        assert_eq!(result.layout, base());
        assert!(matches!(
            result.warnings.as_slice(),
            [PersonalWarning::Parse(_)]
        ));
    }
}
