//! Confluence Cloud REST client, models and storage<->markdown conversion.
//! Contract: docs/ARCHITECTURE.md § "Module: confluence".

pub mod client;
pub mod convert;
pub mod models;

pub use client::{ConfluenceClient, ConfluenceError, Result};
pub use models::{
    Attachment, ConfluenceAuth, PageBrief, PageDetail, PageStorage, PageTreeNode, PageVersion,
    SpaceSummary, TestResult,
};
