use axum::extract::State;
use phantom_core::{Err, Result, err};
use phantom_service::Services;
use ruma::{
    RoomId, UInt, UserId,
    api::client::backup::{
        add_backup_keys, add_backup_keys_for_room, add_backup_keys_for_session,
        create_backup_version, delete_backup_keys, delete_backup_keys_for_room,
        delete_backup_keys_for_session, delete_backup_version, get_backup_info, get_backup_keys,
        get_backup_keys_for_room, get_backup_keys_for_session, get_latest_backup_info,
        update_backup_version,
    },
    serde::Raw,
};

use crate::router::Ruma;

/// # `POST /_matrix/client/r0/room_keys/version`
///
/// Creates a new backup.
pub(crate) async fn create_backup_version_route(
    State(services): State<crate::router::State>,
    body: Ruma<create_backup_version::v3::Request>,
) -> Result<create_backup_version::v3::Response> {
    let version = services
        .key_backups
        .create_backup(body.sender_user(), &body.algorithm)?;

    Ok(create_backup_version::v3::Response::new(version))
}

/// # `PUT /_matrix/client/r0/room_keys/version/{version}`
///
/// Update information about an existing backup. Only `auth_data` can be
/// modified.
pub(crate) async fn update_backup_version_route(
    State(services): State<crate::router::State>,
    body: Ruma<update_backup_version::v3::Request>,
) -> Result<update_backup_version::v3::Response> {
    services
        .key_backups
        .update_backup(body.sender_user(), &body.version, &body.algorithm)
        .await?;

    Ok(update_backup_version::v3::Response::new())
}

/// # `GET /_matrix/client/r0/room_keys/version`
///
/// Get information about the latest backup version.
pub(crate) async fn get_latest_backup_info_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_latest_backup_info::v3::Request>,
) -> Result<get_latest_backup_info::v3::Response> {
    let sender_user = body.sender_user();

    let (version, algorithm) = services
        .key_backups
        .get_latest_backup(sender_user)
        .await
        .map_err(|_| err!(Request(NotFound("Key backup does not exist."))))?;

    let (count, etag) = count_etag(&services, sender_user, &version).await?;

    Ok(get_latest_backup_info::v3::Response::new(
        algorithm, count, etag, version,
    ))
}

/// # `GET /_matrix/client/v3/room_keys/version/{version}`
///
/// Get information about an existing backup.
pub(crate) async fn get_backup_info_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_backup_info::v3::Request>,
) -> Result<get_backup_info::v3::Response> {
    let sender_user = body.sender_user();

    let algorithm = services
        .key_backups
        .get_backup(sender_user, &body.version)
        .await
        .map_err(|_| {
            err!(Request(NotFound(
                "Key backup does not exist at version {:?}",
                body.version
            )))
        })?;

    let (count, etag) = count_etag(&services, sender_user, &body.version).await?;

    Ok(get_backup_info::v3::Response::new(
        algorithm,
        count,
        etag,
        body.version.clone(),
    ))
}

/// # `DELETE /_matrix/client/r0/room_keys/version/{version}`
///
/// Delete an existing key backup.
///
/// - Deletes both information about the backup, as well as all key data
///   related to the backup
pub(crate) async fn delete_backup_version_route(
    State(services): State<crate::router::State>,
    body: Ruma<delete_backup_version::v3::Request>,
) -> Result<delete_backup_version::v3::Response> {
    services
        .key_backups
        .delete_backup(body.sender_user(), &body.version)
        .await;

    Ok(delete_backup_version::v3::Response::new())
}

/// # `PUT /_matrix/client/r0/room_keys/keys`
///
/// Add the received backup keys to the database.
///
/// - Only manipulating the most recently created version of the backup is
///   allowed
/// - Adds the keys to the backup
/// - Returns the new number of keys in this backup and the etag
pub(crate) async fn add_backup_keys_route(
    State(services): State<crate::router::State>,
    body: Ruma<add_backup_keys::v3::Request>,
) -> Result<add_backup_keys::v3::Response> {
    let sender_user = body.sender_user();

    latest_version_check(&services, sender_user, &body.version).await?;

    for (room_id, room) in &body.rooms {
        for (session_id, key_data) in &room.sessions {
            add_key(
                &services,
                sender_user,
                &body.version,
                room_id,
                session_id,
                key_data,
            )
            .await?;
        }
    }

    let (count, etag) = count_etag(&services, sender_user, &body.version).await?;

    Ok(add_backup_keys::v3::Response::new(etag, count))
}

/// # `PUT /_matrix/client/r0/room_keys/keys/{roomId}`
///
/// Add the received backup keys to the database.
///
/// - Only manipulating the most recently created version of the backup is
///   allowed
/// - Adds the keys to the backup
/// - Returns the new number of keys in this backup and the etag
pub(crate) async fn add_backup_keys_for_room_route(
    State(services): State<crate::router::State>,
    body: Ruma<add_backup_keys_for_room::v3::Request>,
) -> Result<add_backup_keys_for_room::v3::Response> {
    let sender_user = body.sender_user();

    latest_version_check(&services, sender_user, &body.version).await?;

    for (session_id, key_data) in &body.sessions {
        add_key(
            &services,
            sender_user,
            &body.version,
            &body.room_id,
            session_id,
            key_data,
        )
        .await?;
    }

    let (count, etag) = count_etag(&services, sender_user, &body.version).await?;

    Ok(add_backup_keys_for_room::v3::Response::new(etag, count))
}

/// # `PUT /_matrix/client/r0/room_keys/keys/{roomId}/{sessionId}`
///
/// Add the received backup key to the database.
///
/// - Only manipulating the most recently created version of the backup is
///   allowed
/// - Adds the keys to the backup
/// - Returns the new number of keys in this backup and the etag
pub(crate) async fn add_backup_keys_for_session_route(
    State(services): State<crate::router::State>,
    body: Ruma<add_backup_keys_for_session::v3::Request>,
) -> Result<add_backup_keys_for_session::v3::Response> {
    let sender_user = body.sender_user();

    latest_version_check(&services, sender_user, &body.version).await?;

    add_key(
        &services,
        sender_user,
        &body.version,
        &body.room_id,
        &body.session_id,
        &body.session_data,
    )
    .await?;

    let (count, etag) = count_etag(&services, sender_user, &body.version).await?;

    Ok(add_backup_keys_for_session::v3::Response::new(etag, count))
}

/// # `GET /_matrix/client/r0/room_keys/keys`
///
/// Retrieves all keys from the backup.
pub(crate) async fn get_backup_keys_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_backup_keys::v3::Request>,
) -> Result<get_backup_keys::v3::Response> {
    let rooms = services
        .key_backups
        .get_all(body.sender_user(), &body.version)
        .await;

    Ok(get_backup_keys::v3::Response::new(rooms))
}

/// # `GET /_matrix/client/r0/room_keys/keys/{roomId}`
///
/// Retrieves all keys from the backup for a given room.
pub(crate) async fn get_backup_keys_for_room_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_backup_keys_for_room::v3::Request>,
) -> Result<get_backup_keys_for_room::v3::Response> {
    let sessions = services
        .key_backups
        .get_room(body.sender_user(), &body.version, &body.room_id)
        .await;

    Ok(get_backup_keys_for_room::v3::Response::new(sessions))
}

/// # `GET /_matrix/client/r0/room_keys/keys/{roomId}/{sessionId}`
///
/// Retrieves a key from the backup.
pub(crate) async fn get_backup_keys_for_session_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_backup_keys_for_session::v3::Request>,
) -> Result<get_backup_keys_for_session::v3::Response> {
    let key_data = services
        .key_backups
        .get_session(
            body.sender_user(),
            &body.version,
            &body.room_id,
            &body.session_id,
        )
        .await
        .map_err(|_| {
            err!(Request(NotFound(debug_error!(
                "Backup key not found for this user's session."
            ))))
        })?;

    Ok(get_backup_keys_for_session::v3::Response::new(key_data))
}

/// # `DELETE /_matrix/client/r0/room_keys/keys`
///
/// Delete the keys from the backup.
pub(crate) async fn delete_backup_keys_route(
    State(services): State<crate::router::State>,
    body: Ruma<delete_backup_keys::v3::Request>,
) -> Result<delete_backup_keys::v3::Response> {
    let sender_user = body.sender_user();

    services
        .key_backups
        .delete_all_keys(sender_user, &body.version)
        .await;

    let (count, etag) = count_etag(&services, sender_user, &body.version).await?;

    Ok(delete_backup_keys::v3::Response::new(etag, count))
}

/// # `DELETE /_matrix/client/r0/room_keys/keys/{roomId}`
///
/// Delete the keys from the backup for a given room.
pub(crate) async fn delete_backup_keys_for_room_route(
    State(services): State<crate::router::State>,
    body: Ruma<delete_backup_keys_for_room::v3::Request>,
) -> Result<delete_backup_keys_for_room::v3::Response> {
    let sender_user = body.sender_user();

    services
        .key_backups
        .delete_room_keys(sender_user, &body.version, &body.room_id)
        .await;

    let (count, etag) = count_etag(&services, sender_user, &body.version).await?;

    Ok(delete_backup_keys_for_room::v3::Response::new(etag, count))
}

/// # `DELETE /_matrix/client/r0/room_keys/keys/{roomId}/{sessionId}`
///
/// Delete a key from the backup.
pub(crate) async fn delete_backup_keys_for_session_route(
    State(services): State<crate::router::State>,
    body: Ruma<delete_backup_keys_for_session::v3::Request>,
) -> Result<delete_backup_keys_for_session::v3::Response> {
    let sender_user = body.sender_user();

    services
        .key_backups
        .delete_room_key(sender_user, &body.version, &body.room_id, &body.session_id)
        .await;

    let (count, etag) = count_etag(&services, sender_user, &body.version).await?;

    Ok(delete_backup_keys_for_session::v3::Response::new(
        etag, count,
    ))
}

/// Refuses writes to any backup but the user's most recent one.
async fn latest_version_check(services: &Services, user_id: &UserId, version: &str) -> Result {
    let latest = services
        .key_backups
        .get_latest_backup_version(user_id)
        .await
        .map_err(|_| err!(Request(NotFound("Key backup does not exist."))))?;

    if latest != version {
        return Err!(Request(InvalidParam(
            "You may only manipulate the most recently created version of the backup."
        )));
    }

    Ok(())
}

/// Stores one session key unless the backup already holds a better one.
///
/// Per the spec, a stored key wins over the new one when it is verified and
/// the new one is not, or when it covers an earlier first message index, or,
/// failing both, when it was forwarded fewer times.
async fn add_key(
    services: &Services,
    user_id: &UserId,
    version: &str,
    room_id: &RoomId,
    session_id: &str,
    key_data: &Raw<ruma::api::client::backup::KeyBackupData>,
) -> Result {
    let existing = services
        .key_backups
        .get_session(user_id, version, room_id, session_id)
        .await;

    if let Ok(existing) = existing
        && let (Ok(old), Ok(new)) = (existing.deserialize(), key_data.deserialize())
    {
        let keep_old = (old.is_verified, new.is_verified) == (true, false)
            || (old.is_verified == new.is_verified
                && (old.first_message_index, old.forwarded_count)
                    < (new.first_message_index, new.forwarded_count));

        if keep_old {
            return Ok(());
        }
    }

    services
        .key_backups
        .add_key(user_id, version, room_id, session_id, key_data)
        .await
}

async fn count_etag(
    services: &Services,
    user_id: &UserId,
    version: &str,
) -> Result<(UInt, String)> {
    let count = services.key_backups.count_keys(user_id, version).await;
    let count =
        UInt::try_from(count).map_err(|_| err!(Request(Unknown("Backup has too many keys."))))?;

    let etag = services.key_backups.get_etag(user_id, version).await?;

    Ok((count, etag))
}
