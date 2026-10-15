//! Fact kinds of the seat entity: `seat.created`, `seat.retired`.
//!
//! No kinds yet: they land in their leaf tasks, each as one `FactKind` type
//! here, one `.with::<K>()` in `FactRegistry::builtins` and one materializer
//! in the table-owning `src/store` file. Handlers call
//! `materialize::require_author` first and then `super::with_input`.
