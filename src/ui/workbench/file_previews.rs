use super::*;
use crate::ui::editor::DocumentId;
use crate::ui::editor::preview::{
    FilePreview, FilePreviewEvent, MAX_PREVIEW_BYTES, decode_preview, is_svg_path,
};

impl WorkbenchView {
    pub(super) fn open_project_file_external(
        &mut self,
        project_id: ProjectId,
        relative_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(services) = self.project.services.get(&project_id).cloned() else {
            return;
        };
        let task =
            cx.background_spawn(async move { services.prepare_external_file(&relative_path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, _, cx| {
                match result {
                    Ok(path) => cx.open_with_system(&path),
                    Err(error) => root.load_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn ensure_file_preview(
        &mut self,
        id: &DocumentId,
        relative_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<FilePreview>> {
        if let Some(preview) = self.project.project_editor_runtime.preview(id) {
            return Some(preview.clone());
        }
        let services = self.project.services.get(&id.project_id)?;
        let download = services.requires_download();
        let text = self.ui_text.clone();
        let preview = cx.new(|cx| FilePreview::new(relative_path, text, download, cx));
        let subscription = cx.subscribe_in(&preview, window, Self::on_file_preview_event);
        self.project.project_editor_runtime.insert_preview(
            id.clone(),
            preview.clone(),
            subscription,
        );
        let session = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&id.project_id)?;
        session.open_file(id.canonical_path.clone());
        let _ = self.select_work_item(WorkItemId::File(id.clone()));
        cx.notify();
        Some(preview)
    }

    pub(super) fn open_unavailable_file(
        &mut self,
        request: &ProjectFileLoadRequest,
        error: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(preview) = self.ensure_file_preview(
            &request.document_id,
            request.relative_path.clone(),
            window,
            cx,
        ) {
            preview.update(cx, |preview, cx| preview.complete(Err(error), cx));
        }
    }

    pub(super) fn open_file_preview(
        &mut self,
        project_id: ProjectId,
        relative_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(services) = self.project.services.get(&project_id).cloned() else {
            return;
        };
        let Some(canonical_path) = services.document_path(&relative_path) else {
            return;
        };
        let id = DocumentId {
            project_id,
            canonical_path,
        };
        let Some(preview) = self.ensure_file_preview(&id, relative_path.clone(), window, cx) else {
            return;
        };
        let generation = self
            .project
            .project_editor_runtime
            .begin_preview_load(id.clone());
        preview.update(cx, |preview, cx| preview.start_loading(cx));
        // SVG source edits must be previewed without saving or losing the document.
        let source = self
            .project
            .project_editor_runtime
            .document(&id)
            .filter(|_| is_svg_path(&relative_path))
            .map(|document| document.read(cx).model().value().as_bytes().to_vec());
        let task = cx.background_spawn(async move {
            let bytes = match source {
                Some(bytes) => bytes,
                None => {
                    let mut bytes = Vec::new();
                    services.copy_file_to(&relative_path, &mut bytes, MAX_PREVIEW_BYTES)?;
                    bytes
                }
            };
            decode_preview(bytes, is_svg_path(&relative_path)).map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, _, cx| {
                if root
                    .project
                    .project_editor_runtime
                    .finish_file_load(&id, generation)
                    && let Some(preview) = root.project.project_editor_runtime.preview(&id).cloned()
                {
                    preview.update(cx, |preview, cx| preview.complete(result, cx));
                }
            });
        })
        .detach();
    }

    fn on_file_preview_event(
        &mut self,
        preview: &Entity<FilePreview>,
        event: &FilePreviewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.project.project_editor_runtime.preview_id(&preview) else {
            return;
        };
        let relative_path = preview.read(cx).relative_path.clone();
        match event {
            FilePreviewEvent::OpenExternal => {
                self.open_project_file_external(id.project_id, relative_path, window, cx)
            }
            FilePreviewEvent::Reload => {
                self.open_file_preview(id.project_id, relative_path, window, cx)
            }
            FilePreviewEvent::ShowSource => {
                self.project.project_editor_runtime.remove_preview(&id);
                self.spawn_project_file_open(id.project_id, relative_path, window, cx);
            }
        }
        cx.notify();
    }
}
