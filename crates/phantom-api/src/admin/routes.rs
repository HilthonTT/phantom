//! The routes this server answers, as it registered them.

use axum::{Extension, Json, response::IntoResponse};
use phantom_core::Result;

use crate::router::{AdminAuth, RouteInfo, RouteTable};

/// # `GET /_phantom/admin/v1/routes`
///
/// Every method and path the router serves, by path, with the audience it
/// serves and, for a route declared through ruma, the endpoint it implements.
pub(super) async fn routes(
    _admin: AdminAuth,
    Extension(table): Extension<RouteTable>,
) -> Result<impl IntoResponse> {
    Ok(Json(sorted(&table.0)))
}

fn sorted(table: &[RouteInfo]) -> Vec<RouteInfo> {
    let mut routes = table.to_vec();
    routes.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.method.cmp(&b.method)));

    routes
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use figment::{
        Figment,
        providers::{Format, Toml},
    };
    use phantom_core::Config;

    use super::sorted;
    use crate::router::Routes;

    fn config(federation: bool) -> Config {
        let toml = format!(
            r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            allow_federation = {federation}
            "#
        );

        Config::new(&Figment::new().merge(Toml::string(&toml).nested())).expect("config is valid")
    }

    /// Builds the router as the server does. axum panics on a method routed
    /// twice on one path, so building it is half the test.
    fn routes(config: &Config) -> Routes {
        let routes = crate::client::register(Routes::new(), config);
        let routes = crate::server::register(routes, config);
        let routes = crate::oidc::register(routes);

        crate::admin::register(routes)
    }

    #[test]
    fn the_route_table_is_complete_and_free_of_duplicates() {
        for federation in [true, false] {
            let routes = routes(&config(federation));
            let table = sorted(routes.table());

            assert!(table.len() > 100, "only {} routes", table.len());

            let mut seen = BTreeSet::new();
            for route in &table {
                assert!(
                    seen.insert((route.method.as_str(), route.path.as_str())),
                    "{} {} is listed twice",
                    route.method,
                    route.path
                );
            }

            for group in [
                "client",
                "media",
                "admin",
                "oidc",
                "well-known",
                "federation",
            ] {
                assert!(
                    table.iter().any(|route| route.group == group),
                    "no {group} routes with federation {federation}"
                );
            }

            assert!(
                table
                    .iter()
                    .any(|route| route.path == "/_phantom/admin/v1/routes" && route.method == "GET"),
                "the routes route lists itself"
            );

            // Building the router is what panics on an overlapping route.
            drop(routes.into_router());
        }
    }
}
