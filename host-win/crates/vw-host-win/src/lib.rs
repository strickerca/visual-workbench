//! Windows app-level platform adapters. No project API stores trust material.

#![cfg(target_os = "windows")]

pub mod routes;
pub mod trust;
