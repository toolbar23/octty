use gpui::{
    Action, App, Application, Bounds, Context, IntoElement, KeyBinding, Menu, MenuItem, Render,
    SharedString, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_component::Root;
use octty_core::{WorkspaceSummary, workspace_shortcut_targets};
use octty_store::{TursoStore, default_store_path};

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct OpenWorkspaceShortcut {
    index: usize,
}

struct OcttyApp {
    status: SharedString,
    workspaces: Vec<WorkspaceSummary>,
}

impl OcttyApp {
    fn new(status: impl Into<SharedString>) -> Self {
        Self {
            status: status.into(),
            workspaces: Vec::new(),
        }
    }
}

impl Render for OcttyApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let shortcuts = workspace_shortcut_targets(&self.workspaces);
        let shortcut_text = if shortcuts.is_empty() {
            "No workspaces discovered yet.".to_owned()
        } else {
            shortcuts
                .iter()
                .map(|target| format!("{} <{}>", target.workspace_id, target.label))
                .collect::<Vec<_>>()
                .join("\n")
        };

        div()
            .flex()
            .size_full()
            .bg(rgb(0x171717))
            .text_color(rgb(0xf2f2f2))
            .child(
                div()
                    .w(px(280.0))
                    .h_full()
                    .border_r_1()
                    .border_color(rgb(0x3a3a3a))
                    .p_4()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("Octty"),
                    )
                    .child(
                        div()
                            .mt_4()
                            .text_sm()
                            .text_color(rgb(0xa0a0a0))
                            .child(shortcut_text),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p_6()
                    .child(div().text_xl().child("Taskspace"))
                    .child(
                        div()
                            .mt_3()
                            .text_sm()
                            .text_color(rgb(0xb8b8b8))
                            .child(self.status.clone()),
                    ),
            )
    }
}

fn main() {
    if std::env::args().any(|arg| arg == "--headless-check") {
        let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
        runtime.block_on(async {
            TursoStore::open(default_store_path())
                .await
                .expect("open Turso store");
        });
        println!("octty-rs headless check ok");
        return;
    }

    Application::new().run(|cx: &mut App| {
        gpui_component::init(cx);
        cx.on_action(open_workspace_shortcut);
        cx.bind_keys(workspace_key_bindings());
        set_workspace_menu(cx, &[]);

        let bounds = Bounds::centered(None, size(px(1200.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Octty".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|_| {
                    OcttyApp::new(
                        "Rust shell scaffold is live. Next slice wires JJ discovery, Turso state, and terminal panes into this GPUI surface.",
                    )
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("open Octty window");
        cx.activate(true);
    });
}

fn set_workspace_menu(cx: &mut App, workspaces: &[WorkspaceSummary]) {
    cx.set_menus(vec![Menu {
        name: "Workspaces".into(),
        items: workspace_menu_items(workspaces),
    }]);
}

fn workspace_menu_items(workspaces: &[WorkspaceSummary]) -> Vec<MenuItem> {
    workspace_shortcut_targets(workspaces)
        .into_iter()
        .enumerate()
        .map(|(index, target)| {
            let workspace = &workspaces[index];
            let name = format!(
                "{} <{}>",
                workspace.display_name_or_workspace_name(),
                target.label
            );
            MenuItem::action(name, OpenWorkspaceShortcut { index })
        })
        .collect()
}

fn workspace_key_bindings() -> [KeyBinding; 10] {
    [
        KeyBinding::new("ctrl-shift-1", OpenWorkspaceShortcut { index: 0 }, None),
        KeyBinding::new("ctrl-shift-2", OpenWorkspaceShortcut { index: 1 }, None),
        KeyBinding::new("ctrl-shift-3", OpenWorkspaceShortcut { index: 2 }, None),
        KeyBinding::new("ctrl-shift-4", OpenWorkspaceShortcut { index: 3 }, None),
        KeyBinding::new("ctrl-shift-5", OpenWorkspaceShortcut { index: 4 }, None),
        KeyBinding::new("ctrl-shift-6", OpenWorkspaceShortcut { index: 5 }, None),
        KeyBinding::new("ctrl-shift-7", OpenWorkspaceShortcut { index: 6 }, None),
        KeyBinding::new("ctrl-shift-8", OpenWorkspaceShortcut { index: 7 }, None),
        KeyBinding::new("ctrl-shift-9", OpenWorkspaceShortcut { index: 8 }, None),
        KeyBinding::new("ctrl-shift-0", OpenWorkspaceShortcut { index: 9 }, None),
    ]
}

fn open_workspace_shortcut(action: &OpenWorkspaceShortcut, _cx: &mut App) {
    eprintln!("workspace shortcut requested: {}", action.index + 1);
}

trait WorkspaceDisplayName {
    fn display_name_or_workspace_name(&self) -> &str;
}

impl WorkspaceDisplayName for WorkspaceSummary {
    fn display_name_or_workspace_name(&self) -> &str {
        if self.display_name.is_empty() {
            &self.workspace_name
        } else {
            &self.display_name
        }
    }
}
