use tauri::{LogicalSize, PhysicalPosition};

// All dimensions here are logical pixels; monitor work areas are physical pixels.
fn initial_size(width: f64, height: f64) -> LogicalSize<f64> {
    LogicalSize::new((width * 0.85).min(1440.0), (height * 0.85).min(960.0))
}

pub fn initialize(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let monitor = window.current_monitor()?.or(window.primary_monitor()?);
    let Some(monitor) = monitor else {
        return Ok(());
    };
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    let size = initial_size(
        area.size.width as f64 / scale,
        area.size.height as f64 / scale,
    );
    window.set_min_size(Some(LogicalSize::new(
        size.width.min(800.0),
        size.height.min(560.0),
    )))?;
    window.set_size(size)?;
    // Use the actual outer size so title bars are included when centering.
    let outer = window.outer_size()?;
    window.set_position(PhysicalPosition::new(
        area.position.x + (area.size.width.saturating_sub(outer.width) / 2) as i32,
        area.position.y + (area.size.height.saturating_sub(outer.height) / 2) as i32,
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_screens_and_scaling_leave_space_around_window() {
        for (width, height, scale) in [
            (1366.0, 728.0, 1.0),
            (1920.0, 1040.0, 1.25),
            (1920.0, 1040.0, 1.5),
            (1280.0, 752.0, 2.0),
        ] {
            let size = initial_size(width / scale, height / scale);
            assert!(size.width * scale <= width * 0.85 + 0.001);
            assert!(size.height * scale <= height * 0.85 + 0.001);
            assert!(size.width > 0.0 && size.height > 0.0);
        }
    }

    #[test]
    fn large_screens_keep_preferred_size() {
        assert_eq!(
            initial_size(3840.0, 2120.0),
            LogicalSize::new(1440.0, 960.0)
        );
    }
}
