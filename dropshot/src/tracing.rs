pub use crate::{dtrace::RequestInfo, dtrace::ResponseInfo};

pub trait Tracing: std::fmt::Debug + Clone + Send + Sync + 'static {
    type Registration: Unpin;

    fn register(&self) -> Self::Registration;
    fn request_start(&self, request_info: &RequestInfo);
    fn request_done(&self, response_info: &ResponseInfo);
}

#[cfg(feature = "usdt-probes")]
#[derive(Debug, Clone, Copy)]
pub struct Dtrace;

#[cfg(feature = "usdt-probes")]
impl Tracing for Dtrace {
    type Registration = crate::dtrace::ProbeRegistration;

    fn register(&self) -> Self::Registration {
        match usdt::register_probes() {
            Ok(()) => crate::dtrace::ProbeRegistration::Succeeded,
            Err(e) => crate::dtrace::ProbeRegistration::Failed(e),
        }
    }

    fn request_start(&self, request_info: &RequestInfo) {
        crate::dtrace::probes::request__start!(|| request_info);
    }

    fn request_done(&self, response_info: &ResponseInfo) {
        crate::dtrace::probes::request_done!(|| response_info);
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Noop;

impl Tracing for Noop {
    type Registration = ();

    fn register(&self) -> Self::Registration {
        ()
    }

    fn request_start(&self, _: &RequestInfo) {}

    fn request_done(&self, _: &ResponseInfo) {}
}
