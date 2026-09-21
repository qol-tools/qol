use qol_audio::devices::{self, Direction};

fn main() -> Result<(), qol_audio::AudioError> {
    for direction in [Direction::Input, Direction::Output] {
        let heading = match direction {
            Direction::Input => "input",
            Direction::Output => "output",
        };
        let default = devices::default_name(direction)?;
        println!(
            "{heading} default: {}",
            default.as_deref().unwrap_or("<none>")
        );
        for device in devices::list(direction)? {
            println!(
                "{heading} device index={} name={} description={} state={:?} monitor_of_sink={:?} active_port={:?}",
                device.index,
                device.name,
                device.description,
                device.state,
                device.monitor_of_sink,
                device.active_port
            );
        }
    }
    Ok(())
}
