//! AI moderation and policy adapter.
//!
//! This module should define a stable interface between forum posting flows and
//! whichever moderation backend is eventually chosen.  Keep the interface small:
//! request in, policy verdict out, optional machine-readable reason.
