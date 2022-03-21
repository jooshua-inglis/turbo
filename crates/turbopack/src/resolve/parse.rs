use std::fmt::Display;

use lazy_static::lazy_static;
use regex::Regex;

#[turbo_tasks::value]
#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub enum Request {
    Relative { path: String },
    Module { module: String, path: String },
    ServerRelative { path: String },
    Windows { path: String },
    Empty,
    PackageInternal { path: String },
    Uri { protocol: String, remainer: String },
    Unknown { path: String },
}

impl Request {
    pub fn request(&self) -> String {
        match self {
            Request::Relative { path } => format!("{path}"),
            Request::Module { module, path } => format!("{module}{path}"),
            Request::ServerRelative { path } => format!("{path}"),
            Request::Windows { path } => format!("{path}"),
            Request::Empty => format!(""),
            Request::PackageInternal { path } => format!("{path}"),
            Request::Uri { protocol, remainer } => format!("{protocol}{remainer}"),
            Request::Unknown { path } => format!("{path}"),
        }
    }
}

#[turbo_tasks::value_impl]
impl RequestRef {
    pub fn parse(request: String) -> Self {
        Self::slot(if request.is_empty() {
            Request::Empty
        } else if request.starts_with("/") {
            Request::ServerRelative { path: request }
        } else if request.starts_with("#") {
            Request::PackageInternal { path: request }
        } else if request.starts_with("./") || request.starts_with("../") {
            Request::Relative { path: request }
        } else {
            lazy_static! {
                static ref WINDOWS_PATH: Regex = Regex::new(r"^([A-Za-z]:\\|\\\\)").unwrap();
                static ref URI_PATH: Regex = Regex::new(r"^([^/\\]+:)(/.+)").unwrap();
                static ref MODULE_PATH: Regex = Regex::new(r"^((?:@[^/]+/)?[^/]+)(.*)").unwrap();
            }
            if WINDOWS_PATH.is_match(&request) {
                return Self::slot(Request::Windows { path: request });
            }
            if let Some(caps) = URI_PATH.captures(&request) {
                if let (Some(protocol), Some(remainer)) = (caps.get(1), caps.get(2)) {
                    // TODO data uri
                    return Self::slot(Request::Uri {
                        protocol: protocol.as_str().to_string(),
                        remainer: remainer.as_str().to_string(),
                    });
                }
            }
            if let Some(caps) = MODULE_PATH.captures(&request) {
                if let (Some(module), Some(path)) = (caps.get(1), caps.get(2)) {
                    return Self::slot(Request::Module {
                        module: module.as_str().to_string(),
                        path: path.as_str().to_string(),
                    });
                }
            }
            Request::Unknown { path: request }
        })
    }
}

impl Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Request::Relative { path } => write!(f, "relative '{}'", path),
            Request::Module { module, path } => {
                if path.is_empty() {
                    write!(f, "module '{}'", module)
                } else {
                    write!(f, "module '{}' with subpath '{}'", module, path)
                }
            }
            Request::ServerRelative { path } => write!(f, "server relative '{}'", path),
            Request::Windows { path } => write!(f, "windows '{}'", path),
            Request::Empty => write!(f, "empty"),
            Request::PackageInternal { path } => write!(f, "package internal '{}'", path),
            Request::Uri { protocol, remainer } => write!(f, "uri '{}' '{}'", protocol, remainer),
            Request::Unknown { path } => write!(f, "unknown '{}'", path),
        }
    }
}