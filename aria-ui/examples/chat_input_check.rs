//! Native smoke check: cargo run --example chat_input_check
//! Opens only the UI, dispatches pointer/keyboard events, and exits without calling the AI.
use std::{cell::RefCell, rc::Rc, time::Duration};

use aria_ui::MascotWindow;
use slint::{platform::{PointerEventButton, WindowEvent}, ComponentHandle, LogicalPosition, LogicalSize};

fn click(ui: &MascotWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    ui.window().dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
    ui.window().dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
}

fn type_text(ui: &MascotWindow, text: &str) {
    for c in text.chars() {
        ui.window().dispatch_event(WindowEvent::KeyPressed { text: c.to_string().into() });
        ui.window().dispatch_event(WindowEvent::KeyReleased { text: c.to_string().into() });
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MascotWindow::new()?;
    ui.set_bar_visible(true);
    ui.set_bubble_shown(false);
    let sent = Rc::new(RefCell::new(Vec::<String>::new()));
    ui.on_send_text({ let sent = sent.clone(); move |text| sent.borrow_mut().push(text.to_string()) });
    ui.show()?;
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(500), move || {
        let ui = weak.unwrap();
        for scale in [0.5_f32, 1.0, 2.5] {
            let stage_w = 260.0 * scale;
            let width = stage_w.max(420.0) + 24.0;
            ui.set_stage_w(stage_w);
            ui.set_stage_h(370.0 * scale);
            ui.window().set_size(LogicalSize::new(width, 370.0 * scale + 54.0));
            ui.set_draft("".into());
            // At minimum size the prompt spans x=190..330. Always inside the text field.
            click(&ui, width / 2.0, 21.0);
            assert!(ui.get_editing(), "click focuses the prompt at scale {scale}");
            type_text(&ui, "hello Aria");
            assert_eq!(ui.get_draft(), "hello Aria", "pointer typing at scale {scale}");
            type_text(&ui, "\n");
            assert_eq!(sent.borrow().last().unwrap(), "hello Aria");
            assert!(ui.get_draft().is_empty(), "Enter submits and clears");
            ui.set_busy(true);
            type_text(&ui, "next prompt");
            let before = sent.borrow().len();
            type_text(&ui, "\n");
            assert_eq!(sent.borrow().len(), before, "busy prevents duplicate sends");
            assert_eq!(ui.get_draft(), "next prompt", "busy preserves editing");
            ui.set_busy(false);
            let bar_w = (width - 24.0).min(520.0);
            click(&ui, (width + bar_w) / 2.0 - 84.0, 21.0);
            assert_eq!(sent.borrow().last().unwrap(), "next prompt", "send button works");
            assert!(ui.get_draft().is_empty());
            ui.invoke_focus_prompt();
            assert!(ui.get_editing(), "opening chat can focus the field");
        }
        println!("Chat input passed: click, typing, Enter, send, busy draft and focus at three scales.");
        slint::quit_event_loop().unwrap();
    });
    slint::run_event_loop()
}
