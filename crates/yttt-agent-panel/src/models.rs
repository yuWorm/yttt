//! The models the picker offers, which one is active and which the user
//! starred. The agent owns the list and the active model and keeps the panel
//! in sync with [`crate::event::ThreadEvent::ModelCatalog`] and
//! [`crate::event::ThreadEvent::ActiveModel`]; a pick here is a request the
//! agent confirms.
use crate::event::{ModelInfo, ModelStatus};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelCatalog {
    models: Vec<ModelInfo>,
    active: Option<String>,
    favorites: Vec<String>,
    /// A refresh was asked for and the new list has not arrived yet.
    syncing: bool,
}

/// A titled group of models in the picker.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelSection<'a> {
    pub title: String,
    pub models: Vec<&'a ModelInfo>,
}

impl ModelCatalog {
    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }

    pub fn active(&self) -> Option<&ModelInfo> {
        let active = self.active.as_deref()?;
        self.models.iter().find(|model| model.id == active)
    }

    pub fn is_syncing(&self) -> bool {
        self.syncing
    }

    pub fn set_syncing(&mut self, syncing: bool) {
        self.syncing = syncing;
    }

    /// Replaces the list and ends a sync. The active model stays while it
    /// is listed; otherwise the first available model takes over.
    pub fn set_models(&mut self, models: Vec<ModelInfo>) {
        self.models = models;
        self.syncing = false;
        if self.active().is_none() {
            self.active = self
                .models
                .iter()
                .find(|model| model.status == ModelStatus::Available)
                .map(|model| model.id.clone());
        }
    }

    /// The user's pick: only a listed, available model.
    pub fn select(&mut self, id: &str) -> bool {
        let available = self
            .models
            .iter()
            .any(|model| model.id == id && model.status == ModelStatus::Available);
        if available {
            self.active = Some(id.to_owned());
        }
        available
    }

    /// The agent's word on what it uses, which wins even over a model the
    /// picker shows as unusable.
    pub fn set_active(&mut self, id: &str) -> bool {
        let listed = self.models.iter().any(|model| model.id == id);
        if listed {
            self.active = Some(id.to_owned());
        }
        listed
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.iter().any(|favorite| favorite == id)
    }

    pub fn set_favorite(&mut self, id: &str, favorite: bool) {
        self.favorites.retain(|existing| existing != id);
        if favorite {
            self.favorites.push(id.to_owned());
        }
    }

    pub fn set_favorites(&mut self, ids: Vec<String>) {
        self.favorites = ids;
    }

    pub fn favorites(&self) -> &[String] {
        &self.favorites
    }

    /// The models whose name or provider contains `query`, ignoring case:
    /// favorites first, then one section per provider in list order.
    pub fn sections(&self, query: &str) -> Vec<ModelSection<'_>> {
        let query = query.trim().to_lowercase();
        let matches = |model: &ModelInfo| {
            query.is_empty()
                || model.name.to_lowercase().contains(&query)
                || model.provider.to_lowercase().contains(&query)
        };
        let favorites = self
            .favorites
            .iter()
            .filter_map(|id| self.models.iter().find(|model| &model.id == id))
            .filter(|model| matches(model))
            .collect::<Vec<_>>();
        let mut groups: Vec<ModelSection<'_>> = Vec::new();
        for model in &self.models {
            if !matches(model) || self.is_favorite(&model.id) {
                continue;
            }
            match groups
                .iter_mut()
                .find(|group| group.title == model.provider)
            {
                Some(group) => group.models.push(model),
                None => groups.push(ModelSection {
                    title: model.provider.clone(),
                    models: vec![model],
                }),
            }
        }
        let mut sections = Vec::new();
        if !favorites.is_empty() {
            sections.push(ModelSection {
                title: "Favorites".to_owned(),
                models: favorites,
            });
        }
        sections.extend(groups);
        sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ModelCapabilities;

    fn model(id: &str, provider: &str, status: ModelStatus) -> ModelInfo {
        ModelInfo {
            id: id.to_owned(),
            name: id.to_owned(),
            provider: provider.to_owned(),
            context_window: None,
            capabilities: ModelCapabilities::default(),
            status,
            icon: None,
        }
    }

    fn catalog() -> ModelCatalog {
        let mut catalog = ModelCatalog::default();
        catalog.set_models(vec![
            model("setup", "Cloud", ModelStatus::NeedsSetup),
            model("large", "Cloud", ModelStatus::Available),
            model("local", "Local", ModelStatus::Unavailable("off".to_owned())),
            model("fast", "Cloud", ModelStatus::Available),
        ]);
        catalog
    }

    fn titles(catalog: &ModelCatalog, query: &str) -> Vec<(String, Vec<String>)> {
        catalog
            .sections(query)
            .into_iter()
            .map(|section| {
                (
                    section.title,
                    section
                        .models
                        .iter()
                        .map(|model| model.id.clone())
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn the_first_available_model_becomes_active() {
        assert_eq!(
            catalog().active().map(|model| model.id.as_str()),
            Some("large")
        );
    }

    #[test]
    fn a_new_list_keeps_a_listed_active_model_and_ends_the_sync() {
        let mut catalog = catalog();
        assert!(catalog.select("fast"));
        catalog.set_syncing(true);
        catalog.set_models(vec![
            model("large", "Cloud", ModelStatus::Available),
            model("fast", "Cloud", ModelStatus::Available),
        ]);
        assert_eq!(
            catalog.active().map(|model| model.id.as_str()),
            Some("fast")
        );
        assert!(!catalog.is_syncing());
        catalog.set_models(vec![model("large", "Cloud", ModelStatus::Available)]);
        assert_eq!(
            catalog.active().map(|model| model.id.as_str()),
            Some("large")
        );
    }

    #[test]
    fn the_user_picks_only_usable_models_but_the_agent_decides() {
        let mut catalog = catalog();
        assert!(!catalog.select("setup"));
        assert!(!catalog.select("missing"));
        assert_eq!(
            catalog.active().map(|model| model.id.as_str()),
            Some("large")
        );
        assert!(catalog.set_active("setup"));
        assert!(!catalog.set_active("missing"));
        assert_eq!(
            catalog.active().map(|model| model.id.as_str()),
            Some("setup")
        );
    }

    #[test]
    fn sections_put_favorites_first_and_group_the_rest_by_provider() {
        let mut catalog = catalog();
        catalog.set_favorite("fast", true);
        assert_eq!(
            titles(&catalog, ""),
            [
                ("Favorites".to_owned(), vec!["fast".to_owned()]),
                (
                    "Cloud".to_owned(),
                    vec!["setup".to_owned(), "large".to_owned()]
                ),
                ("Local".to_owned(), vec!["local".to_owned()]),
            ]
        );
        catalog.set_favorite("fast", false);
        assert_eq!(titles(&catalog, "").len(), 2);
    }

    #[test]
    fn a_query_matches_names_and_providers_ignoring_case() {
        let catalog = catalog();
        assert_eq!(
            titles(&catalog, "LAR"),
            [("Cloud".to_owned(), vec!["large".to_owned()])]
        );
        assert_eq!(
            titles(&catalog, "local"),
            [("Local".to_owned(), vec!["local".to_owned()])]
        );
        assert!(titles(&catalog, "zzz").is_empty());
    }
}
