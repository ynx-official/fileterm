use anyhow::Context as _;
use gpui::{App, Global, WindowHandle};
use tokio::sync::mpsc;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use crate::{
    view::RootView,
    window::menu::{OpenCommandManager, OpenConnectionManager},
};

const SHOW_MAIN: &str = "fileterm-tray-show-main";
const CONNECTIONS: &str = "fileterm-tray-connections";
const COMMANDS: &str = "fileterm-tray-commands";
const QUIT: &str = "fileterm-tray-quit";

#[derive(Clone, Copy)]
enum TrayCommand {
    ShowMain,
    Connections,
    Commands,
    Quit,
}

pub struct TrayHandler {
    _icon: TrayIcon,
}

impl Global for TrayHandler {}

impl TrayHandler {
    pub fn setup(
        is_english: bool,
        main_window: WindowHandle<RootView>,
        cx: &mut App,
    ) -> anyhow::Result<Self> {
        let label = |en: &'static str, zh: &'static str| -> &'static str {
            if is_english {
                en
            } else {
                zh
            }
        };
        let menu = Menu::new();
        let show = MenuItem::with_id(
            SHOW_MAIN,
            label("Show FileTerm", "显示 FileTerm"),
            true,
            None,
        );
        let connections = MenuItem::with_id(
            CONNECTIONS,
            label("Connection Manager", "连接管理器"),
            true,
            None,
        );
        let commands =
            MenuItem::with_id(COMMANDS, label("Command Manager", "命令管理器"), true, None);
        let separator = PredefinedMenuItem::separator();
        let quit = MenuItem::with_id(QUIT, label("Quit FileTerm", "退出 FileTerm"), true, None);
        menu.append_items(&[&show, &connections, &commands, &separator, &quit])
            .context("build FileTerm tray menu")?;

        let icon = TrayIconBuilder::new()
            .with_tooltip("FileTerm")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_icon(fileterm_icon()?)
            .build()
            .context("create FileTerm tray icon")?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        let tray_sender = sender.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = tray_sender.send(TrayCommand::ShowMain);
            }
        }));
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = match event.id.as_ref() {
                SHOW_MAIN => Some(TrayCommand::ShowMain),
                CONNECTIONS => Some(TrayCommand::Connections),
                COMMANDS => Some(TrayCommand::Commands),
                QUIT => Some(TrayCommand::Quit),
                _ => None,
            };
            if let Some(command) = command {
                let _ = sender.send(command);
            }
        }));

        let mut async_cx = cx.to_async();
        cx.foreground_executor()
            .spawn(async move {
                while let Some(command) = receiver.recv().await {
                    let _ = main_window.update(&mut async_cx, |_, window, cx| match command {
                        TrayCommand::ShowMain => window.activate_window(),
                        TrayCommand::Connections => {
                            window.activate_window();
                            window.dispatch_action(Box::new(OpenConnectionManager), cx);
                        }
                        TrayCommand::Commands => {
                            window.activate_window();
                            window.dispatch_action(Box::new(OpenCommandManager), cx);
                        }
                        TrayCommand::Quit => cx.quit(),
                    });
                }
            })
            .detach();

        Ok(Self { _icon: icon })
    }
}

fn fileterm_icon() -> anyhow::Result<Icon> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let index = ((y * SIZE + x) * 4) as usize;
            let inside = (3..29).contains(&x) && (3..29).contains(&y);
            let mark = ((8..13).contains(&x) && (8..24).contains(&y))
                || ((8..23).contains(&x) && (8..12).contains(&y))
                || ((8..20).contains(&x) && (15..19).contains(&y))
                || ((20..25).contains(&x) && (15..24).contains(&y));
            let (red, green, blue, alpha) = if mark {
                (255, 255, 255, 255)
            } else if inside {
                (37, 99, 235, 255)
            } else {
                (0, 0, 0, 0)
            };
            rgba[index..index + 4].copy_from_slice(&[red, green, blue, alpha]);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).context("build FileTerm tray icon")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_icon_is_valid() {
        assert!(fileterm_icon().is_ok());
    }
}
