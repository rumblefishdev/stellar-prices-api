//! The API hub's dashboard contract (task 0348).
//!
//! The hub (`rumblefishdev/sorobanscan-api-portal`) holds the developer's
//! Discord session and calls Prices server-to-server, one call per action,
//! each with a token of its own: `v1.<payload>.<mac>`, an HMAC-SHA256 under a
//! secret the two sides share through AWS Secrets Manager (the shared sign-in
//! spec's decisions 12 and 15). This module verifies that token
//! ([`token::verify`]) and parses the secret ([`secret::HubSecret`]). The
//! secret is loaded by [`crate::portal::sources::HubSource`], on the first hub
//! call that needs it and apart from the portal's own sources: a missing or
//! bad one closes the contract only, never the portal, and a failed read is
//! tried again after a short cooldown.

pub mod secret;
pub mod token;
