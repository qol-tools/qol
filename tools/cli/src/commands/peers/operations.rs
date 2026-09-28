use anyhow::{anyhow, bail, Result};
use qol_headless::{Command, PlainTextOutput};
use qol_peers::operations::{Request, Response, MAX_BODY_BYTES};
use qol_runtime::PlatformStateClient;
use std::io::Read;

pub(super) fn attach(mut command: Command) -> Command {
    for (name, arguments, detail) in [
        ("invoke", " PEER", "Freeze one OperationBody from bounded stdin, allocate once, and send once."),
        ("outcome", "", "Query the original RequestHandle from bounded stdin without replay."),
        ("reconcile", "", "Query the original RequestHandle from bounded stdin without replay."),
        ("cancel", "", "Cancel the original RequestHandle from bounded stdin. DispatchStarted cannot be undone."),
        ("requests", " PEER", "List bounded request handles, including any unresolved sender allocation."),
    ] {
        command = command.subcommand(Command::new(name)
            .about(detail)
            .usage(format!("qol peers {name}{arguments} [--json]"))
            .detail("Help works offline. Acknowledged means handler acknowledgement. Unknown is an uncertain physical outcome; never invoke a replacement to recover it. Use requests to recover a handle after a lost local reply. Arguments and handles are stdin JSON, never command-line diagnostics.")
            .run_plain_text(move |context| Ok(PlainTextOutput::text(serde_json::to_string(&run(name, context.args())?)? + "\n")))
            .run_json(move |context| Ok(serde_json::to_value(run(name, context.args())?)?)));
    }
    command
}

fn run(name: &str, args: &[String]) -> Result<Response> {
    let input = parse(name, args, &mut std::io::stdin().lock())?;
    let runtime = PlatformStateClient::from_env();
    let expected =
        super::authority(&mut |request| runtime.peer_admin(request).map_err(anyhow::Error::new))?
            .expected();
    let request = match input {
        Input::Invoke(peer, body) => Request::Invoke {
            expected,
            peer,
            body,
        },
        Input::Outcome(handle) => Request::Outcome { expected, handle },
        Input::Cancel(handle) => Request::Cancel { expected, handle },
        Input::Requests(peer) => Request::Requests { expected, peer },
    };
    let response = runtime.peer_operation(request)?;
    if matches!(response, Response::Error { .. } | Response::Unknown { .. }) {
        bail!("{}", serde_json::to_string(&response)?);
    }
    Ok(response)
}

enum Input {
    Invoke(qol_peers::PeerId, String),
    Outcome(qol_peers::operations::RequestHandle),
    Cancel(qol_peers::operations::RequestHandle),
    Requests(qol_peers::PeerId),
}

fn parse(name: &str, args: &[String], stdin: &mut impl Read) -> Result<Input> {
    match (name, args) {
        ("invoke", [peer]) => {
            let peer = peer
                .parse()
                .map_err(|_| anyhow!("invalid peer identifier"))?;
            let mut bytes = Vec::new();
            stdin
                .take((MAX_BODY_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| anyhow!("cannot read operation stdin"))?;
            if bytes.len() > MAX_BODY_BYTES {
                bail!("operation body exceeds byte limit");
            }
            let body =
                String::from_utf8(bytes).map_err(|_| anyhow!("operation body must be UTF-8"))?;
            serde_json::from_str::<qol_peers::operations::OperationBody>(&body)
                .map_err(|_| anyhow!("invalid operation body"))?;
            Ok(Input::Invoke(peer, body))
        }
        ("outcome" | "reconcile", []) => Ok(Input::Outcome(super::input::read_json(stdin)?)),
        ("cancel", []) => Ok(Input::Cancel(super::input::read_json(stdin)?)),
        ("requests", [peer]) => Ok(Input::Requests(
            peer.parse()
                .map_err(|_| anyhow!("invalid peer identifier"))?,
        )),
        _ => bail!("invalid peer operation arguments; use help"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_cli_rejects_unbounded_stdin_and_never_echoes_arguments() {
        for (name, args, document) in [
            (
                "invoke",
                vec!["A".repeat(43)],
                "x".repeat(MAX_BODY_BYTES + 1),
            ),
            ("cancel", vec![], "secret-canary".into()),
            ("outcome", vec!["secret-canary".into()], "{}".into()),
        ] {
            let error = parse(name, &args, &mut document.as_bytes())
                .err()
                .unwrap()
                .to_string();
            assert!(!error.contains("secret-canary"), "{name}");
        }
    }
}
