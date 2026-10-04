use std::{any::Any, future::Future};

use axum::{
    Router,
    body::Body,
    extract::{FromRequest, FromRequestParts},
    handler::Handler,
    response::{IntoResponse, Response},
    routing::{MethodFilter, on},
};
use http::Request;
use ruma::api::{IncomingRequest, path_builder::PathBuilder};

use super::{Authenticate, Ruma, RumaResponse, State};

pub trait RouterExt {
    #[must_use]
    fn ruma_route<H, T>(self, handler: &'static H) -> Self
    where
        H: RumaHandler<T>;
}

impl RouterExt for Router<State> {
    fn ruma_route<H, T>(self, handler: &'static H) -> Self
    where
        H: RumaHandler<T>,
    {
        handler.register(self)
    }
}

pub trait RumaHandler<T> {
    fn register(&'static self, router: Router<State>) -> Router<State>;

    /// Runs the handler for one request.
    ///
    /// The future is opaque on purpose: axum's `Handler` impl below then needs
    /// only this declared `Send` bound, so codegen never has to prove `Send`
    /// for each concrete handler future. rustc (1.96, 1.97) fails at that with
    /// an ICE in `Instance::expect_resolve` when the future awaits an opaque
    /// `impl Stream + Send`/`impl Future + Send` from a trait method.
    fn call_route(
        handler: RouteHandler,
        state: State,
        request: Request<Body>,
    ) -> impl Future<Output = Response> + Send + 'static;
}

struct Route<Fut> {
    call: RouteCall<Fut>,
    handler: RouteHandler,
}

type RouteCall<Fut> = fn(RouteHandler, State, Request<Body>) -> Fut;

type RouteHandler = &'static (dyn Any + Send + Sync);

impl<Fut> Clone for Route<Fut> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Fut> Copy for Route<Fut> {}

impl<Fut> Handler<(), State> for Route<Fut>
where
    Fut: Future<Output = Response> + Send + 'static,
{
    type Future = Fut;

    fn call(self, request: Request<Body>, state: State) -> Self::Future {
        (self.call)(self.handler, state, request)
    }
}

macro_rules! ruma_handler {
    ($($extractor:ident),*) => {
        #[allow(non_snake_case)]
        impl<F, Fut, Req, Err, const ADMIN: bool, $($extractor,)*> RumaHandler<($($extractor,)* Ruma<Req, ADMIN>,)> for F
        where
            F: Fn($($extractor,)* Ruma<Req, ADMIN>) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Result<Req::OutgoingResponse, Err>> + Send + 'static,
            Req: IncomingRequest + Send + Sync + 'static,
            Req::Authentication: Authenticate,
            Req::OutgoingResponse: Send,
            Err: IntoResponse + Send,
            $($extractor: FromRequestParts<State> + Send + 'static,)*
        {
            fn register(&'static self, router: Router<State>) -> Router<State> {
                let filter = MethodFilter::try_from(Req::METHOD)
                    .expect("ruma endpoints only use routable HTTP methods");

                let route = Route { handler: self, call: Self::call_route };

                Req::PATH_BUILDER
                    .all_paths()
                    .fold(router, |router, path| router.route(path, on(filter, route)))
            }

            fn call_route(
                handler: RouteHandler,
                state: State,
                request: Request<Body>,
            ) -> impl Future<Output = Response> + Send + 'static {
                let handler: &'static F = handler
                    .downcast_ref()
                    .expect("route handler matches the type it registered with");

                let response = async move {
                    #[allow(unused_mut)]
                    let (mut parts, body) = request.into_parts();

                    $(
                        let $extractor = match $extractor::from_request_parts(&mut parts, &state).await {
                            Ok(value) => value,
                            Err(rejection) => return rejection.into_response(),
                        };
                    )*

                    let request = Request::from_parts(parts, body);
                    let args = match Ruma::<Req, ADMIN>::from_request(request, &state).await {
                        Ok(args) => args,
                        Err(rejection) => return rejection.into_response(),
                    };

                    match handler($($extractor,)* args).await {
                        Ok(response) => RumaResponse(response).into_response(),
                        Err(error) => error.into_response(),
                    }
                };

                // Debug builds keep handler futures off the stack.
                #[cfg(debug_assertions)]
                let response = Box::pin(response);

                response
            }
        }
    };
}

ruma_handler!();
ruma_handler!(T1);
ruma_handler!(T1, T2);
ruma_handler!(T1, T2, T3);
ruma_handler!(T1, T2, T3, T4);
