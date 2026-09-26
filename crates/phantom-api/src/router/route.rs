use std::{future::Future, pin::Pin};

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
}

struct RumaRoute<F: 'static>(&'static F);

impl<F> Clone for RumaRoute<F> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<F> Copy for RumaRoute<F> {}

type BoxedResponse = Pin<Box<dyn Future<Output = Response> + Send>>;

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

                Req::PATH_BUILDER
                    .all_paths()
                    .fold(router, |router, path| router.route(path, on(filter, RumaRoute(self))))
            }
        }

        #[allow(non_snake_case)]
        impl<F, Fut, Req, Err, const ADMIN: bool, $($extractor,)*> Handler<($($extractor,)* Ruma<Req, ADMIN>,), State> for RumaRoute<F>
        where
            F: Fn($($extractor,)* Ruma<Req, ADMIN>) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Result<Req::OutgoingResponse, Err>> + Send + 'static,
            Req: IncomingRequest + Send + Sync + 'static,
            Req::Authentication: Authenticate,
            Req::OutgoingResponse: Send,
            Err: IntoResponse + Send,
            $($extractor: FromRequestParts<State> + Send + 'static,)*
        {
            type Future = BoxedResponse;

            fn call(self, request: Request<Body>, state: State) -> Self::Future {
                Box::pin(async move {
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

                    match (self.0)($($extractor,)* args).await {
                        Ok(response) => RumaResponse(response).into_response(),
                        Err(error) => error.into_response(),
                    }
                })
            }
        }
    };
}

ruma_handler!();
ruma_handler!(T1);
ruma_handler!(T1, T2);
ruma_handler!(T1, T2, T3);
ruma_handler!(T1, T2, T3, T4);
