use std::collections::{HashSet, VecDeque};

use phantom_core::{Err, Result, implement};
use ruma::{OwnedRoomId, OwnedServerName, RoomId, api::client::space::SpaceHierarchyRoomsChunk};

use super::{Asker, Service, SummaryAccessibility, token::PaginationToken};
use crate::rooms::short::ShortRoomId;

pub struct PagedHierarchy {
    pub rooms: Vec<SpaceHierarchyRoomsChunk>,

    pub next_batch: Option<String>,
}

struct Visit {
    short: ShortRoomId,
    children: VecDeque<(OwnedRoomId, Vec<OwnedServerName>)>,
}

#[implement(Service)]
pub async fn client_hierarchy(
    &self,
    asker: Asker<'_>,
    room_id: &RoomId,
    limit: u64,
    max_depth: u64,
    suggested_only: bool,
    from: Option<&str>,
) -> Result<PagedHierarchy> {
    let resume = from.map(PaginationToken::decode).transpose()?;

    let parameters = PaginationToken {
        path: Vec::new(),
        limit,
        max_depth,
        suggested_only,
    };

    if resume
        .as_ref()
        .is_some_and(|resume| !resume.same_parameters(&parameters))
    {
        return Err!(Request(InvalidParam(
            "limit, max_depth and suggested_only cannot change while paginating."
        )));
    }

    let target = resume.map(|resume| resume.path);
    let mut emitting = target.is_none();

    let via = room_id
        .server_name()
        .map(ToOwned::to_owned)
        .into_iter()
        .collect::<Vec<_>>();

    let root = match self.summary(room_id, asker, &via, suggested_only).await {
        Some(SummaryAccessibility::Accessible(chunk)) => *chunk,
        Some(SummaryAccessibility::Inaccessible) => {
            return Err!(Request(Forbidden("You are not allowed to see this room.")));
        }
        None => {
            return Err!(Request(NotFound("The room is unknown to this server.")));
        }
    };

    let root_short = self.services.short.get_or_create_shortroomid(room_id).await;

    let mut rooms: Vec<SpaceHierarchyRoomsChunk> = Vec::new();
    let mut next_batch = None;
    let mut visited: HashSet<ShortRoomId> = HashSet::from([root_short]);
    let mut stack: Vec<Visit> = Vec::new();

    let mut current = Some((root_short, root, Vec::new()));

    'walk: loop {
        let (short, summary, path) = match current.take() {
            Some(current) => current,
            None => {
                let Some((room, via)) = next_child(&mut stack) else {
                    break 'walk;
                };

                let short = self.services.short.get_or_create_shortroomid(&room).await;

                if !visited.insert(short) {
                    continue 'walk;
                }

                let Some(SummaryAccessibility::Accessible(summary)) =
                    self.summary(&room, asker, &via, suggested_only).await
                else {
                    continue 'walk;
                };

                let summary = *summary;

                let path = stack.iter().map(|visit| visit.short).collect();

                (short, summary, path)
            }
        };

        let path: Vec<ShortRoomId> = path.into_iter().chain([short]).collect();

        emitting = emitting || target.as_deref() == Some(path.as_slice());

        if emitting {
            if rooms.len() as u64 >= limit {
                next_batch = Some(
                    PaginationToken {
                        path,
                        limit,
                        max_depth,
                        suggested_only,
                    }
                    .to_string(),
                );

                break 'walk;
            }

            rooms.push(summary.clone());
        }

        if u64::try_from(stack.len()).unwrap_or(u64::MAX) < max_depth {
            stack.push(Visit {
                short,
                children: self.children_of(&summary, suggested_only).into(),
            });
        }
    }

    Ok(PagedHierarchy { rooms, next_batch })
}

fn next_child(stack: &mut Vec<Visit>) -> Option<(OwnedRoomId, Vec<OwnedServerName>)> {
    while let Some(visit) = stack.last_mut() {
        match visit.children.pop_front() {
            Some(child) => return Some(child),
            None => {
                stack.pop();
            }
        }
    }

    None
}
