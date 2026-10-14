//! The router as it is built, with a record of every route registered on it.
//!
//! axum keeps no list of its routes that can be read back, so the admin API's
//! route table is taken down as each route is added: [`Routes`] stands in for
//! `Router` in the `register` functions, and [`get`], [`post`] and the rest
//! stand in for axum's, so a route cannot be served without being listed. The
//! console's API section used to keep a list of its own by hand, which went
//! stale without anything noticing.

use std::{any::type_name, sync::Arc};

use axum::{
    Extension, Router,
    handler::Handler,
    routing::{self, MethodRouter},
};
use http::Method;
use serde::Serialize;

use super::{RumaHandler, State};

/// A router being built, and the routes registered on it so far.
pub struct Routes {
    router: Router<State>,
    table: Vec<RouteInfo>,
}

/// One method on one path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RouteInfo {
    /// The HTTP method, or `*` for a route answering any.
    pub method: String,

    /// The path as axum matches it, captures in braces.
    pub path: String,

    /// Who the route is for: "client", "federation", "media", "admin",
    /// "oidc" or "well-known".
    pub group: &'static str,

    /// The ruma request type a route was declared with, as
    /// `membership::join_room_by_id::v3`; none for a route phantom declares
    /// itself.
    pub endpoint: Option<String>,

    /// Whether the path is a stable one rather than an `unstable` prefix.
    pub stable: bool,
}

/// The finished table, handed to the admin API's routes handler as a request
/// extension.
#[derive(Clone, Debug)]
pub struct RouteTable(pub Arc<[RouteInfo]>);

/// What a ruma handler serves, for the table.
pub struct Endpoint {
    pub method: Method,
    pub paths: Vec<&'static str>,

    /// The ruma request type's full name.
    pub request: &'static str,
}

impl Default for Routes {
    fn default() -> Self {
        Self::new()
    }
}

impl Routes {
    #[must_use]
    pub fn new() -> Self {
        Self {
            router: Router::new(),
            table: Vec::new(),
        }
    }

    #[must_use]
    pub fn route(mut self, path: &str, methods: Methods) -> Self {
        self.table.extend(
            methods
                .methods
                .iter()
                .map(|method| RouteInfo::new(method, path, None)),
        );
        self.router = self.router.route(path, methods.router);

        self
    }

    #[must_use]
    pub fn ruma_route<H, T>(mut self, handler: &'static H) -> Self
    where
        H: RumaHandler<T>,
    {
        let endpoint = H::endpoint();
        let name = endpoint_name(endpoint.request);

        self.table.extend(
            endpoint
                .paths
                .iter()
                .map(|path| RouteInfo::new(endpoint.method.as_str(), path, Some(name.clone()))),
        );
        self.router = handler.register(self.router);

        self
    }

    /// Adds another set of routes, as `Router::merge`.
    #[must_use]
    pub fn merge(mut self, other: Self) -> Self {
        self.table.extend(other.table);
        self.router = self.router.merge(other.router);

        self
    }

    /// Changes the router without adding routes, as for a `route_layer`
    /// wrapping the routes so far.
    #[must_use]
    pub fn map_router<F>(mut self, f: F) -> Self
    where
        F: FnOnce(Router<State>) -> Router<State>,
    {
        self.router = f(self.router);
        self
    }

    /// The routes registered so far.
    #[must_use]
    pub fn table(&self) -> &[RouteInfo] {
        &self.table
    }

    /// The router, carrying its own route table for the admin API to answer
    /// with.
    pub fn into_router(self) -> Router<State> {
        let table = RouteTable(self.table.into());

        self.router.layer(Extension(table))
    }
}

impl RouteInfo {
    fn new(method: &str, path: &str, endpoint: Option<String>) -> Self {
        Self {
            method: method.to_owned(),
            path: path.to_owned(),
            group: group(path),
            endpoint,
            stable: !path.contains("/unstable/"),
        }
    }
}

/// Which audience a path serves, read from its prefix. Media is told apart
/// from the client and federation APIs it is part of, since an operator
/// thinks of it, and proxies it, separately.
#[must_use]
pub fn group(path: &str) -> &'static str {
    const MEDIA: [&str; 3] = [
        "/_matrix/media/",
        "/_matrix/client/v1/media/",
        "/_matrix/federation/v1/media/",
    ];

    match path {
        _ if MEDIA.iter().any(|prefix| path.starts_with(prefix)) => "media",
        _ if path.starts_with("/.well-known/") => "well-known",
        _ if path.starts_with("/_matrix/federation/") || path.starts_with("/_matrix/key/") => {
            "federation"
        }
        _ if path.starts_with("/_phantom/admin/") => "admin",
        _ if path.starts_with("/_phantom/oidc/")
            || path.ends_with("/auth_issuer")
            || path.ends_with("/auth_metadata") =>
        {
            "oidc"
        }
        _ => "client",
    }
}

/// A ruma request type's name without its crate and the `Request` every one
/// ends in: `ruma_client_api::membership::join_room_by_id::v3::Request` reads
/// `membership::join_room_by_id::v3`.
fn endpoint_name(request: &str) -> String {
    let name = request.strip_suffix("::Request").unwrap_or(request);

    match name.split_once("::") {
        Some((krate, rest)) if krate.starts_with("ruma") => rest.to_owned(),
        _ => name.to_owned(),
    }
}

/// The methods one path answers, as axum's `MethodRouter`, recording which
/// they are as they are added.
pub struct Methods {
    router: MethodRouter<State>,
    methods: Vec<&'static str>,
}

macro_rules! method {
    ($($name:ident => $method:literal),* $(,)?) => {
        $(
            #[doc = concat!("Routes `", $method, "` to `handler`, as axum's `", stringify!($name), "`.")]
            #[must_use]
            pub fn $name<H, T>(handler: H) -> Methods
            where
                H: Handler<T, State>,
                T: 'static,
            {
                Methods {
                    router: routing::$name(handler),
                    methods: vec![$method],
                }
            }
        )*

        impl Methods {
            $(
                #[doc = concat!("Also routes `", $method, "` to `handler`.")]
                #[must_use]
                pub fn $name<H, T>(mut self, handler: H) -> Self
                where
                    H: Handler<T, State>,
                    T: 'static,
                {
                    self.router = self.router.$name(handler);
                    self.methods.push($method);
                    self
                }
            )*
        }
    };
}

method! {
    get => "GET",
    post => "POST",
    put => "PUT",
    delete => "DELETE",
    patch => "PATCH",
}

/// Routes every method to `handler`, as axum's `any`.
#[must_use]
pub fn any<H, T>(handler: H) -> Methods
where
    H: Handler<T, State>,
    T: 'static,
{
    Methods {
        router: routing::any(handler),
        methods: vec!["*"],
    }
}

/// The name a ruma request type is listed under; split out for the macro in
/// `route.rs`, which knows the type.
#[must_use]
pub(super) fn request_name<Req>() -> &'static str {
    type_name::<Req>()
}

#[cfg(test)]
mod tests {
    use super::{endpoint_name, group};

    #[test]
    fn groups_follow_the_path_prefix() {
        assert_eq!(group("/_matrix/client/v3/sync"), "client");
        assert_eq!(group("/_matrix/client/v1/media/download/{a}/{b}"), "media");
        assert_eq!(group("/_matrix/media/v3/config"), "media");
        assert_eq!(group("/_matrix/federation/v1/send/{txn_id}"), "federation");
        assert_eq!(group("/_matrix/federation/v1/media/download/{id}"), "media");
        assert_eq!(group("/_matrix/key/v2/server"), "federation");
        assert_eq!(group("/_phantom/admin/v1/routes"), "admin");
        assert_eq!(group("/_phantom/oidc/token"), "oidc");
        assert_eq!(group("/_matrix/client/v1/auth_metadata"), "oidc");
        assert_eq!(group("/.well-known/matrix/client"), "well-known");
        assert_eq!(group("/_phantom/server_version"), "client");
    }

    #[test]
    fn endpoint_names_drop_the_crate_and_request() {
        assert_eq!(
            endpoint_name("ruma_client_api::membership::join_room_by_id::v3::Request"),
            "membership::join_room_by_id::v3"
        );
        assert_eq!(endpoint_name("phantom::Thing"), "phantom::Thing");
    }
}
