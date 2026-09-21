#[cfg(target_os = "linux")]
use qol_audio::control;
#[cfg(target_os = "linux")]
use qol_audio::default_output;
#[cfg(target_os = "linux")]
use qol_audio::devices::Direction;

#[cfg(target_os = "linux")]
fn main() -> Result<(), qol_audio::AudioError> {
    let facts = control::server_facts()?;
    println!(
        "server name={:?} version={:?} incarnation={} default_sink={:?} default_source={:?}",
        facts.name, facts.version, facts.incarnation, facts.default_sink, facts.default_source
    );

    for card in control::list_cards()? {
        println!(
            "card index={} name={} description={} driver={:?} active_profile={:?}",
            card.index, card.name, card.description, card.driver, card.active_profile
        );
        for profile in card.profiles {
            println!(
                "card profile name={} description={} availability={:?} priority={}",
                profile.name, profile.description, profile.availability, profile.priority
            );
        }
    }

    for sink in control::list_sinks()? {
        println!(
            "sink index={} name={} description={} state={:?} muted={} monitor_source={:?}",
            sink.index,
            sink.name,
            sink.description,
            sink.state,
            sink.muted,
            sink.monitor_source_index
        );
    }

    for source in control::list_sources()? {
        println!(
            "source index={} name={} description={} state={:?} muted={} monitor_of_sink={:?}",
            source.index,
            source.name,
            source.description,
            source.state,
            source.muted,
            source.monitor_of_sink_index
        );
    }

    for output in control::list_source_outputs()? {
        println!(
            "source output index={} name={} source={} corked={} client={:?}",
            output.index, output.name, output.source_index, output.corked, output.client_index
        );
    }

    let default = default_output::effective(Direction::Output)?;
    println!("default output: {}", default.as_deref().unwrap_or("<none>"));
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("the audio server inventory is unavailable on this platform");
}
