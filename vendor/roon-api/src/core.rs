use std::sync::Arc;

use roon_moo::connection::MooConnection;

use crate::browse::Browse;
use crate::image::ImageService;
use crate::transport::Transport;

/// A handle to a connected and registered Roon Core.
///
/// Provides access to services (transport, browse, image) available on the core.
/// This type is cheaply cloneable.
#[derive(Debug, Clone)]
pub struct Core {
    pub(crate) inner: Arc<CoreInner>,
}

#[derive(Debug)]
pub(crate) struct CoreInner {
    pub(crate) core_id: String,
    pub(crate) display_name: String,
    pub(crate) display_version: String,
    #[allow(dead_code)]
    pub(crate) token: Option<String>,
    pub(crate) http_port: u16,
    pub(crate) host: String,
    pub(crate) connection: Arc<MooConnection>,
}

impl Core {
    pub fn core_id(&self) -> &str {
        &self.inner.core_id
    }

    pub fn display_name(&self) -> &str {
        &self.inner.display_name
    }

    pub fn display_version(&self) -> &str {
        &self.inner.display_version
    }

    pub fn transport(&self) -> Transport {
        Transport::new(self.inner.connection.clone())
    }

    pub fn browse(&self) -> Browse {
        Browse::new(self.inner.connection.clone())
    }

    /// Get the Image service for retrieving album art and other images.
    pub fn image(&self) -> ImageService {
        ImageService::new(&self.inner.host, self.inner.http_port)
    }

    pub fn is_alive(&self) -> bool {
        self.inner.connection.is_alive()
    }
}
