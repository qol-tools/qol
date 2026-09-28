mod enrollment;
mod input;
mod operations;
mod pagination;

#[cfg(test)]
mod tests;

use anyhow::{anyhow, bail, Result};
use qol_headless::{Command, PlainTextOutput};
use qol_peers::admin::{AuthoritySummary, Request, Response, Status};
use qol_runtime::PlatformStateClient;

use input::Action;

const OUTPUT: &str = "Canonical qol-peers admin::Response values: one JSON object per line by default, or a JSON array with --json. Typed Error responses are canonical JSON on stderr and exit nonzero. Failed pagination emits no partial snapshot.";
const DETAILS: &str = "Uses the existing local tray runtime without starting it. Convenience mutations read Status once, then submit once; stale authority stamps and unknown outcomes are never retried. list and grants follow one snapshot. request dispatches one canonical admin::Request from bounded stdin; set-grants reads a canonical OperationKey array. prepare and redeem read the raw invitation document from bounded stdin, never arguments. Only invite exports a secret. A queued attempt is not a bilateral link; inspect pending, approve explicitly, then inspect attempt. Help works offline.";

type Runner = fn(Option<&str>, &[String]) -> Result<Vec<Response>>;

pub(crate) fn command() -> Command {
    command_with_runner(run)
}

fn command_with_runner(run: Runner) -> Command {
    let mut command = handlers(
        Command::new("peers")
            .about("Inspect and control the local peer authority.")
            .usage("qol peers <status|list|grants|session|create|open|stop|rename|revoke|set-grants|network|invite|cancel-invitation|pending|approve|reject|prepare|redeem|recover|abandon|resume|outbound|attempt|request|pointz|invoke|outcome|reconcile|cancel|requests> [--json]")
            .detail(DETAILS),
        None,
        run,
    );
    for (name, arguments, about) in [
        ("status", "", "Read peer authority status."),
        (
            "network",
            "",
            "Read normal and enrollment listener readiness.",
        ),
        (
            "invite",
            " ADDRESS...",
            "Create an invitation and explicitly export its secret document.",
        ),
        ("cancel-invitation", " INVITATION", "Cancel an invitation."),
        (
            "pending",
            "",
            "Inspect authenticated pending peers and both storage lifetimes.",
        ),
        (
            "approve",
            " INVITATION TRANSACTION PEER",
            "Approve the presented request with empty grants.",
        ),
        (
            "reject",
            " INVITATION TRANSACTION PEER",
            "Reject the presented request by cancelling its invitation.",
        ),
        (
            "prepare",
            "",
            "Prepare a durable original transaction using invitation stdin.",
        ),
        (
            "redeem",
            " TRANSACTION",
            "Queue redemption of the original transaction using invitation stdin.",
        ),
        (
            "recover",
            " TRANSACTION ENDPOINT...",
            "Queue recovery using the stored inviter pin and literal endpoints.",
        ),
        (
            "abandon",
            " TRANSACTION",
            "Abandon locally without claiming remote rollback.",
        ),
        (
            "resume",
            " TRANSACTION",
            "Resume the original transaction without sending it.",
        ),
        (
            "outbound",
            "",
            "Read bounded durable outbound pages from one snapshot.",
        ),
        (
            "attempt",
            " TRANSACTION",
            "Read transient progress; unavailable after owner restart.",
        ),
        ("list", "", "Read all peer pages from one fresh snapshot."),
        ("grants", " PEER", "Read all grant pages for one peer."),
        (
            "session",
            " NAME",
            "Start an explicitly requested session authority.",
        ),
        (
            "create",
            " NAME",
            "Create an explicitly requested persistent authority.",
        ),
        ("open", "", "Open the existing persistent authority."),
        (
            "stop",
            "",
            "Stop the current authority using its fresh stamp.",
        ),
        (
            "rename",
            " NAME",
            "Rename the authority using its fresh stamp.",
        ),
        (
            "revoke",
            " PEER",
            "Revoke one peer using the fresh authority stamp.",
        ),
        (
            "set-grants",
            " PEER",
            "Replace grants with a canonical OperationKey array from stdin.",
        ),
        (
            "request",
            "",
            "Dispatch one canonical admin::Request JSON document from stdin.",
        ),
        (
            "pointz",
            " <status|devices|pair|cancel|remove DEVICE>",
            "Inspect paired PointZ phones, open or cancel a pairing window, or remove a phone.",
        ),
    ] {
        command = command.subcommand(handlers(
            Command::new(name)
                .about(about)
                .usage(format!("qol peers {name}{arguments} [--json]"))
                .detail(DETAILS),
            Some(name),
            run,
        ));
    }
    operations::attach(command)
}

fn handlers(command: Command, name: Option<&'static str>, run: Runner) -> Command {
    command
        .output(OUTPUT)
        .exit_behavior("Zero on success; nonzero on invalid input, refusal, stale snapshot, unavailable transport or unknown mutation outcome.")
        .run_plain_text(move |context| {
            let responses = run(name, context.args())?;
            let mut output = String::new();
            for response in responses {
                let bytes = qol_runtime::local_ipc::encode_secret_json(&response)
                    .map_err(|_| anyhow!("peer output exceeds the local message boundary"))?;
                output.push_str(std::str::from_utf8(&bytes).map_err(|_| anyhow!("invalid peer output"))?);
            }
            Ok(PlainTextOutput::text(output))
        })
        .run_json(move |context| Ok(serde_json::to_value(run(name, context.args())?)?))
}

fn run(name: Option<&str>, args: &[String]) -> Result<Vec<Response>> {
    let args: Vec<&str> = name
        .into_iter()
        .chain(args.iter().map(String::as_str))
        .collect();
    let action = input::parse(&args, &mut std::io::stdin().lock())?;
    let runtime = PlatformStateClient::from_env();
    let mut client = |request| runtime.peer_admin(request).map_err(anyhow::Error::new);
    execute(action, &mut client)
}

fn execute(
    action: Action,
    client: &mut impl FnMut(Request) -> Result<Response>,
) -> Result<Vec<Response>> {
    match action {
        Action::Direct(request) => Ok(vec![dispatch(client, request)?]),
        Action::Pages(kind) => pagination::collect(kind, client),
        Action::Start(request) => {
            status(client)?;
            Ok(vec![dispatch(client, request)?])
        }
        Action::Mutation(mutation) => {
            let expected = authority(client)?.expected();
            Ok(vec![dispatch(client, mutation.request(expected))?])
        }
    }
}

fn dispatch(
    client: &mut impl FnMut(Request) -> Result<Response>,
    request: Request,
) -> Result<Response> {
    let response = client(request)?;
    if let Response::Error { .. } = &response {
        bail!("{}", serde_json::to_string(&response)?);
    }
    Ok(response)
}

fn status(client: &mut impl FnMut(Request) -> Result<Response>) -> Result<Status> {
    let Response::Status { status } = dispatch(client, Request::Status)? else {
        bail!("peer administration returned an unexpected status response");
    };
    Ok(status)
}

fn authority(client: &mut impl FnMut(Request) -> Result<Response>) -> Result<AuthoritySummary> {
    status(client)?
        .authority
        .ok_or_else(|| anyhow!("peer authority is unavailable; inspect qol peers status"))
}
