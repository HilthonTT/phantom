#[macro_export]
macro_rules! Err {
	($($args:tt)*) => {
		Err($crate::err!($($args)*))
	};
}

#[macro_export]
macro_rules! err {
	(Request(Forbidden($level:ident!($($args:tt)+)))) => {{
		let mut buf = String::new();
		$crate::error::Error::Request(
			$crate::ruma::api::error::ErrorKind::Forbidden,
			$crate::err_log!(buf, $level, $($args)+),
			$crate::http::StatusCode::BAD_REQUEST
		)
	}};

	(Request(Forbidden($($args:tt)+))) => {
		$crate::error::Error::Request(
			$crate::ruma::api::error::ErrorKind::Forbidden,
			$crate::format_maybe!($($args)+),
			$crate::http::StatusCode::BAD_REQUEST
		)
	};

	(Request($variant:ident($level:ident!($($args:tt)+)))) => {{
		let mut buf = String::new();
		$crate::error::Error::Request(
			$crate::ruma::api::error::ErrorKind::$variant,
			$crate::err_log!(buf, $level, $($args)+),
			$crate::http::StatusCode::BAD_REQUEST
		)
	}};

	(Request($variant:ident($($args:tt)+))) => {
		$crate::error::Error::Request(
			$crate::ruma::api::error::ErrorKind::$variant,
			$crate::format_maybe!($($args)+),
			$crate::http::StatusCode::BAD_REQUEST
		)
	};

	(Config($item:literal, $fmt:literal $(, $($arg:tt)+)?)) => {{
		let mut buf = String::new();
		$crate::error::Error::Config($item, $crate::err_log!(
			buf,
			error,
			message = ::std::format_args!($fmt $(, $($arg)+)?)
		))
	}};

	($variant:ident($level:ident!($($args:tt)+))) => {{
		let mut buf = String::new();
		$crate::error::Error::$variant($crate::err_log!(buf, $level, $($args)+))
	}};

	($variant:ident($($args:ident),+)) => {
		$crate::error::Error::$variant($($args),+)
	};

	($variant:ident($($args:tt)+)) => {
		$crate::error::Error::$variant($crate::format_maybe!($($args)+))
	};

	($level:ident!($($args:tt)+)) => {{
		let mut buf = String::new();
		$crate::error::Error::Err($crate::err_log!(buf, $level, $($args)+))
	}};

	($($args:tt)+) => {
		$crate::error::Error::Err($crate::format_maybe!($($args)+))
	};
}

#[macro_export]
macro_rules! err_log {
	($out:ident, $level:ident, $fmt:literal $(, $($arg:tt)+)?) => {
		$crate::err_log!(@fields $out, $level, message = ::std::format_args!($fmt $(, $($arg)+)?))
	};

	($out:ident, $level:ident, $($fields:tt)+) => {
		$crate::err_log!(@fields $out, $level, $($fields)+)
	};

	(@fields $out:ident, $level:ident, $($fields:tt)+) => {{
		use $crate::tracing::{
			callsite, callsite2, metadata, valueset, Callsite,
			Level,
		};

		const LEVEL: Level = $crate::err_lev!($level);
		static __CALLSITE: callsite::DefaultCallsite = callsite2! {
			name: std::concat! {
				"event ",
				std::file!(),
				":",
				std::line!(),
			},
			kind: metadata::Kind::EVENT,
			target: std::module_path!(),
			level: LEVEL,
			fields: $($fields)+,
		};

		($crate::error::visit)(&mut $out, LEVEL, &__CALLSITE, &mut valueset!(__CALLSITE.metadata().fields(), $($fields)+));
		($out).into()
	}}
}

#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! err_lev {
    (debug_warn) => {
        if $crate::log::debug::logging() {
            $crate::tracing::Level::WARN
        } else {
            $crate::tracing::Level::DEBUG
        }
    };

    (debug_error) => {
        if $crate::log::debug::logging() {
            $crate::tracing::Level::ERROR
        } else {
            $crate::tracing::Level::DEBUG
        }
    };

    (warn) => {
        $crate::tracing::Level::WARN
    };

    (error) => {
        $crate::tracing::Level::ERROR
    };
}

use std::{fmt, fmt::Write};

use tracing::{
    __macro_support, __tracing_log, Callsite, Event, Level,
    callsite::DefaultCallsite,
    field::{Field, ValueSet, Visit},
    level_enabled,
};

struct Visitor<'a>(&'a mut String);

impl Visit for Visitor<'_> {
    #[inline]
    fn record_debug(&mut self, field: &Field, val: &dyn fmt::Debug) {
        if field.name() == "message" {
            write!(self.0, "{val:?}").expect("stream error");
        } else {
            write!(self.0, " {}={val:?}", field.name()).expect("stream error");
        }
    }
}

pub fn visit(
    out: &mut String,
    level: Level,
    __callsite: &'static DefaultCallsite,
    vs: &mut ValueSet<'_>,
) {
    let meta = __callsite.metadata();
    let enabled = level_enabled!(level) && {
        let interest = __callsite.interest();
        !interest.is_never() && __macro_support::__is_enabled(meta, interest)
    };

    if enabled {
        Event::dispatch(meta, vs);
    }

    __tracing_log!(level, __callsite, vs);
    vs.record(&mut Visitor(out));
}
