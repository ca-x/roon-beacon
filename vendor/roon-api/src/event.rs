use crate::core::Core;

/// Events emitted by the Roon SDK during discovery and connection lifecycle.
#[derive(Debug, Clone)]
pub enum RoonEvent {
    /// A Roon Core was discovered on the network.
    CoreFound {
        core_id: String,
        display_name: String,
    },
    /// Successfully paired with a Roon Core. The `Core` handle provides
    /// access to services (transport, browse, etc.).
    CorePaired(Core),
    /// Unpaired from a previously paired Roon Core.
    CoreUnpaired { core_id: String },
    /// A previously discovered Roon Core is no longer reachable.
    CoreLost { core_id: String },
}
