use std::cmp::Ordering;

use futures::{FutureExt, StreamExt, future::join};
use phantom_core::stream::{BroadbandExt, ReadyExt};
use phantom_service::Services;
use ruma::{MxcUri, OwnedMxcUri, RoomId, UserId, api::client::sync::sync_events::v5::response};

const MAX_HEROES: usize = 5;

#[tracing::instrument(name = "heroes", level = "trace", skip_all)]
pub(super) async fn calculate_heroes(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
    room_name: Option<&str>,
    room_avatar: Option<&MxcUri>,
) -> (
    Option<Vec<response::Hero>>,
    Option<String>,
    Option<OwnedMxcUri>,
) {
    let heroes: Vec<response::Hero> = services
        .rooms
        .state_cache
        .room_members(room_id)
        .ready_filter(|&member| member != sender_user)
        .ready_filter_map(|member| room_name.is_none().then_some(member))
        .map(ToOwned::to_owned)
        .broadn_filter_map(MAX_HEROES, async |user_id| {
            let content = services
                .rooms
                .state_accessor
                .get_member(room_id, &user_id)
                .await
                .ok()?;

            let name = async {
                match content.displayname {
                    Some(name) => Some(name),
                    None => services.profile.displayname(&user_id).await.ok(),
                }
            };

            let avatar = async {
                match content.avatar_url {
                    Some(avatar) => Some(avatar),
                    None => services.profile.avatar_url(&user_id).await.ok(),
                }
            };

            let (name, avatar) = join(name, avatar).boxed().await;
            let mut hero = response::Hero::new(user_id);
            hero.name = name;
            hero.avatar = avatar;

            Some(hero)
        })
        .take(MAX_HEROES)
        .collect()
        .await;

    let hero_name = |hero: &response::Hero| {
        hero.name
            .clone()
            .unwrap_or_else(|| hero.user_id.to_string())
    };

    let heroes_name = match heroes.len().cmp(&(1_usize)) {
        Ordering::Less => None,
        Ordering::Equal => Some(hero_name(&heroes[0])),
        Ordering::Greater => {
            let firsts = heroes[1..]
                .iter()
                .map(hero_name)
                .collect::<Vec<_>>()
                .join(", ");

            let last = hero_name(&heroes[0]);

            Some(format!("{firsts} and {last}"))
        }
    };

    let heroes_avatar = (room_avatar.is_none() && room_name.is_none())
        .then(|| heroes.first().and_then(|hero| hero.avatar.clone()))
        .flatten();

    (Some(heroes), heroes_name, heroes_avatar)
}
