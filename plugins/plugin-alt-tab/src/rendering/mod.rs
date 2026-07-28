pub(crate) mod image_registry;
pub(crate) mod preview_image;
#[cfg(debug_assertions)]
pub(crate) mod preview_trace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RenderingFlow {
    preview_renderer: PreviewRenderer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewRenderer {
    GpuiSnapshots,
    ExternalPreviewPlane { backend: &'static str },
}

impl RenderingFlow {
    pub(crate) fn current() -> Self {
        if let Some(backend) = crate::preview_plane::live_preview_replacement() {
            return Self::external_preview_plane(backend);
        }
        Self::gpui_snapshots()
    }

    pub(crate) fn gpui_snapshots() -> Self {
        Self {
            preview_renderer: PreviewRenderer::GpuiSnapshots,
        }
    }

    pub(crate) fn external_preview_plane(backend: &'static str) -> Self {
        Self {
            preview_renderer: PreviewRenderer::ExternalPreviewPlane { backend },
        }
    }

    pub(crate) fn trace_show(self, _show_id: u64) {
        qol_runtime::probe!(
            "RENDERING_FLOW",
            "show_id={_show_id} preview_renderer={} backend={} gpui_preview_images={} on_open_capture={} live_capture={} preview_fill_capture={}",
            self.preview_renderer_name(),
            self.backend_name(),
            self.renders_gpui_preview_images(),
            self.captures_on_open(),
            self.captures_live_selection(),
            self.captures_preview_fill(),
        );
    }

    pub(crate) fn renders_gpui_preview_images(self) -> bool {
        matches!(self.preview_renderer, PreviewRenderer::GpuiSnapshots)
    }

    pub(crate) fn captures_on_open(self) -> bool {
        self.renders_gpui_preview_images()
    }

    pub(crate) fn captures_live_selection(self) -> bool {
        self.renders_gpui_preview_images()
    }

    pub(crate) fn captures_preview_fill(self) -> bool {
        self.renders_gpui_preview_images()
    }

    pub(crate) fn preview_plane_backend(self) -> Option<&'static str> {
        match self.preview_renderer {
            PreviewRenderer::GpuiSnapshots => None,
            PreviewRenderer::ExternalPreviewPlane { backend } => Some(backend),
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn preview_renderer_name(self) -> &'static str {
        match self.preview_renderer {
            PreviewRenderer::GpuiSnapshots => "gpui_snapshots",
            PreviewRenderer::ExternalPreviewPlane { .. } => "external_preview_plane",
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn backend_name(self) -> &'static str {
        self.preview_plane_backend().unwrap_or("none")
    }
}

#[cfg(test)]
mod tests {
    use super::RenderingFlow;

    #[test]
    fn gpui_snapshot_flow_owns_images_and_capture() {
        let flow = RenderingFlow::gpui_snapshots();

        assert!(flow.renders_gpui_preview_images());
        assert!(flow.captures_on_open());
        assert!(flow.captures_live_selection());
        assert!(flow.captures_preview_fill());
        assert_eq!(flow.preview_plane_backend(), None);
    }

    #[test]
    fn external_preview_plane_flow_suppresses_gpui_capture() {
        let flow = RenderingFlow::external_preview_plane("cinnamon_shell");

        assert!(!flow.renders_gpui_preview_images());
        assert!(!flow.captures_on_open());
        assert!(!flow.captures_live_selection());
        assert!(!flow.captures_preview_fill());
        assert_eq!(flow.preview_plane_backend(), Some("cinnamon_shell"));
    }
}
