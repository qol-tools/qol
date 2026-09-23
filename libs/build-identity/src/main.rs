use qol_build_identity::BuildIdentityEnvironment;
use std::io::Write;

fn main() {
    if let Err(error) = run() {
        eprintln!("qol-build-identity: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let operation = args
        .next()
        .ok_or("usage: qol-build-identity <emit|verify> <production|development|sandbox>")?;
    let intent = args
        .next()
        .ok_or("usage: qol-build-identity <emit|verify> <production|development|sandbox>")?;
    if args.next().is_some() {
        return Err(
            "usage: qol-build-identity <emit|verify> <production|development|sandbox>".into(),
        );
    }
    let repo = std::env::current_dir()?;
    let identity = match intent.as_str() {
        "production" => BuildIdentityEnvironment::production(&repo)?,
        "development" => BuildIdentityEnvironment::development(&repo)?,
        "sandbox" => BuildIdentityEnvironment::sandbox(&repo)?,
        _ => return Err(format!("unsupported build intent {intent:?}").into()),
    };
    match operation.as_str() {
        "emit" => {
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            for (name, value) in identity.variables() {
                writeln!(stdout, "{name}={value}")?;
            }
        }
        "verify" => identity.verify_inherited_environment()?,
        _ => return Err(format!("unsupported operation {operation:?}").into()),
    }
    Ok(())
}
