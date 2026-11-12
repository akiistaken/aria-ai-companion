//! Native layout regression check: cargo run --example bubble_layout_check
use std::{io::Write, time::Duration};
use aria_ui::MascotWindow;
use slint::{ComponentHandle, LogicalSize};

fn main() -> Result<(), slint::PlatformError> {
    let ui = MascotWindow::new()?;
    ui.set_bubble_shown(true);
    ui.show()?;
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(300), move || {
        let ui = weak.unwrap();
        let long = "A longer reply should fit comfortably above my head, wrapping into several lines without cutting off the last sentence. The bubble follows me when I sit down or lean to the side.";
        for scale in [0.5, 1.0, 2.5] {
            ui.set_stage_w(260.0 * scale);
            ui.set_stage_h(370.0 * scale);
            let width = (260.0_f32 * scale).max(420.0) + 24.0;
            ui.window().set_size(LogicalSize::new(width, 370.0 * scale + 400.0));
            ui.set_bubble_text("Hello!".into());
            ui.set_bubble_layout_text("Hello!".into());
            let short_height = ui.get_bubble_h();
            ui.set_bubble_layout_text(long.into());
            let full_height = ui.get_bubble_h();
            assert!(full_height > short_height + 60.0, "wrapped text must grow the bubble");
            assert!(ui.get_bubble_w() <= 232.0);
            let stage_y = ui.get_stage_y();
            for x in [0.0, 0.5, 1.0] {
                for y in [0.0, 0.12, 0.55] {
                    ui.set_head_x(x);
                    ui.set_head_y(y);
                    assert!(ui.get_bubble_x() >= 0.0);
                    assert!(ui.get_bubble_x() + ui.get_bubble_w() <= width);
                    let crown_y = stage_y + y * ui.get_stage_h();
                    assert!((crown_y - ui.get_bubble_y() - full_height - 8.0).abs() < 0.1);
                    assert_eq!(ui.get_stage_y(), stage_y, "tracking must not move the stage");
                }
            }
            for count in [0, 12, long.len()] {
                ui.set_bubble_text(long.chars().take(count).collect::<String>().into());
                assert_eq!(ui.get_bubble_h(), full_height, "typing must not resize the window");
            }
        }
        ui.set_stage_w(260.0);
        ui.set_stage_h(370.0);
        ui.set_head_x(0.5);
        ui.set_head_y(0.12);
        ui.window().set_size(LogicalSize::new(444.0, 370.0 + ui.get_bubble_slot_height() + 54.0));
        let snapshot = ui.window().take_snapshot().expect("UI snapshot");
        std::fs::create_dir_all("target/review-ui").unwrap();
        let mut out = std::fs::File::create("target/review-ui/bubble.ppm").unwrap();
        write!(out, "P6\n{} {}\n255\n", snapshot.width(), snapshot.height()).unwrap();
        for p in snapshot.as_slice() {
            out.write_all(&[p.r, p.g, p.b]).unwrap();
        }
        println!("Bubble passed: wrapping, typing, head tracking and bounds at three scales.");
        slint::quit_event_loop().unwrap();
    });
    slint::run_event_loop()
}
