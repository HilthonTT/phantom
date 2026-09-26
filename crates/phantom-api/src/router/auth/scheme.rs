use std::future::Future;

use phantom_core::{Err, Error, Result};
use phantom_service::Services;
use ruma::api::{
    auth_scheme::{
        AccessToken, AccessTokenOptional, AppserviceToken, AppserviceTokenOptional, AuthScheme,
        NoAccessToken, NoAuthentication,
    },
    error::{ErrorKind, UnknownTokenErrorData},
    federation::authentication::ServerSignatures,
};

use super::{Auth, RawRequest, Token, appservice, federation};

pub trait Authenticate: AuthScheme {
    fn authenticate(
        services: &Services,
        request: &RawRequest,
        token: Token,
    ) -> impl Future<Output = Result<Auth>> + Send;
}

impl Authenticate for NoAuthentication {
    async fn authenticate(_: &Services, _: &RawRequest, token: Token) -> Result<Auth> {
        Ok(anonymous(token))
    }
}

impl Authenticate for NoAccessToken {
    async fn authenticate(_: &Services, _: &RawRequest, token: Token) -> Result<Auth> {
        Ok(anonymous(token))
    }
}

impl Authenticate for AccessToken {
    async fn authenticate(services: &Services, request: &RawRequest, token: Token) -> Result<Auth> {
        match token {
            Token::None => Err!(Request(MissingToken("Missing access token."))),
            Token::Invalid => unknown_token(),
            Token::User(user_id, device_id) => Ok(Auth::user(user_id, device_id)),
            Token::Appservice(info) => appservice::authenticate(services, request, *info).await,
        }
    }
}

impl Authenticate for AccessTokenOptional {
    async fn authenticate(_: &Services, _: &RawRequest, token: Token) -> Result<Auth> {
        optional(token)
    }
}

impl Authenticate for AppserviceToken {
    async fn authenticate(_: &Services, _: &RawRequest, token: Token) -> Result<Auth> {
        match token {
            Token::None => Err!(Request(MissingToken("Missing access token."))),
            Token::Invalid => unknown_token(),
            Token::User(..) => {
                Err!(Request(Unauthorized(
                    "Appservice tokens must be used on this endpoint."
                )))
            }
            Token::Appservice(info) => Ok(Auth::appservice(*info)),
        }
    }
}

impl Authenticate for AppserviceTokenOptional {
    async fn authenticate(_: &Services, _: &RawRequest, token: Token) -> Result<Auth> {
        optional(token)
    }
}

impl Authenticate for ServerSignatures {
    async fn authenticate(services: &Services, request: &RawRequest, token: Token) -> Result<Auth> {
        match token {
            Token::None => federation::authenticate(services, request).await,
            Token::Invalid => unknown_token(),
            Token::User(..) | Token::Appservice(_) => {
                Err!(Request(Unauthorized(
                    "Server signatures must be used on this endpoint."
                )))
            }
        }
    }
}

fn anonymous(token: Token) -> Auth {
    match token {
        Token::None | Token::Invalid => Auth::default(),
        Token::User(user_id, device_id) => Auth::user(user_id, device_id),
        Token::Appservice(info) => Auth::appservice(*info),
    }
}

fn optional(token: Token) -> Result<Auth> {
    match token {
        Token::Invalid => unknown_token(),
        token => Ok(anonymous(token)),
    }
}

fn unknown_token() -> Result<Auth> {
    Err(Error::BadRequest(
        ErrorKind::UnknownToken(UnknownTokenErrorData::new()),
        "Unknown access token.",
    ))
}
