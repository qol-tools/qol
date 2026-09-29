use anyhow::{bail, Result};
use qol_peers::admin::{Page, PageCursor, Request, Response};
use qol_peers::PeerId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Pages {
    Peers,
    Outbound,
    Grants(PeerId),
    Phones,
}

impl Pages {
    fn request(self, cursor: PageCursor) -> Request {
        match self {
            Self::Outbound => Request::Enrollment {
                request: qol_peers::admin::EnrollmentRequest::Outbound { cursor },
            },
            Self::Peers => Request::Peers { cursor },
            Self::Grants(peer_id) => Request::Grants { peer_id, cursor },
            Self::Phones => Request::Pointz {
                request: qol_peers::admin::PointzRequest::Devices { cursor },
            },
        }
    }

    fn next(
        self,
        response: &Response,
        cursor: PageCursor,
        total: &mut Option<u32>,
    ) -> Result<Option<PageCursor>> {
        match (self, response) {
            (Self::Outbound, Response::OutboundEnrollments { page }) => {
                validate(page, cursor, total)
            }
            (Self::Peers, Response::Peers { page }) => validate(page, cursor, total),
            (Self::Grants(expected), Response::Grants { peer_id, page })
                if expected == *peer_id =>
            {
                validate(page, cursor, total)
            }
            (Self::Phones, Response::PointzDevices { page }) => validate(page, cursor, total),
            _ => bail!("peer administration returned an unexpected page response"),
        }
    }
}

pub(super) fn collect(
    pages: Pages,
    client: &mut impl FnMut(Request) -> Result<Response>,
) -> Result<Vec<Response>> {
    let authority = super::authority(client)?;
    let mut cursor = PageCursor {
        authority_id: authority.peer_id,
        activation_id: authority.activation_id,
        revision: authority.revision,
        offset: 0,
    };
    let mut total = None;
    let mut responses = Vec::new();
    loop {
        let response = super::dispatch(client, pages.request(cursor))?;
        let next = pages.next(&response, cursor, &mut total)?;
        responses.push(response);
        let Some(next) = next else {
            return Ok(responses);
        };
        cursor = next;
    }
}

fn validate<T>(
    page: &Page<T>,
    cursor: PageCursor,
    total: &mut Option<u32>,
) -> Result<Option<PageCursor>> {
    if page.cursor != cursor || total.is_some_and(|total| total != page.total) {
        bail!("peer pagination snapshot changed; no pages were emitted");
    }
    let end = u32::try_from(page.items.len())
        .ok()
        .and_then(|count| cursor.offset.checked_add(count));
    let Some(end) = end.filter(|end| *end <= page.total) else {
        bail!("peer pagination returned invalid bounds");
    };
    let expected_next = PageCursor {
        offset: end,
        ..cursor
    };
    match page.next {
        Some(next) if end > cursor.offset && end < page.total && next == expected_next => {}
        None if end == page.total => {}
        _ => bail!("peer pagination returned an invalid continuation cursor"),
    }
    *total = Some(page.total);
    Ok(page.next)
}
