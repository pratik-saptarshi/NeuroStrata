//! Runtime provisioning for Laya, embedded in the daemon and exposed by the `setup`
//! subcommand to bootstrap the local Python/laya-train environment without manual user
//! steps. Requests use the non-blocking session API.