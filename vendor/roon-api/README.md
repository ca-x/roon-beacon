# roon-rs SDK adapter

Source: `shin1ohno/roon-rs`, published `roon-api` 0.5.3, MIT OR Apache-2.0.
The upstream v0.5.7 repository tag still declares this crate as 0.5.3.

Beacon owns discovery, the long-lived registry authorization subscription,
secure persistence and deterministic teardown. `attach_registered` lets the
upstream client reuse that already-authenticated connection instead of creating
another discovery/reconnect loop. Transport commands validate the Roon response
status. Nested outputs accept the protocol's omitted `zone_id`.

Other SDK APIs are retained for source compatibility; Beacon does not use the
SDK's independent discovery/reconnect manager.
