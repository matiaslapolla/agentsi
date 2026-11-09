//! The catalog of methods and events. The OpenRPC schema and the router of
//! the core both read it.

use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A request that a client sends and the core answers.
pub trait Method: 'static {
    /// `namespace.name`, for example `core.hello`.
    const NAME: &'static str;
    const DESCRIPTION: &'static str;
    /// The kinds of error that this method can return, apart from `invalid`
    /// and `internal`, which each method can return.
    const ERRORS: &'static [ErrorKind];
    type Params: Serialize + DeserializeOwned + JsonSchema + Send + 'static;
    type Result: Serialize + DeserializeOwned + JsonSchema + Send + 'static;
}

/// A notification that the core sends to each subscriber of its topic.
pub trait Event: 'static {
    /// `topic.name`, for example `terminal.state`. Events of the `core`
    /// namespace go to each subscriber.
    const NAME: &'static str;
    const DESCRIPTION: &'static str;
    type Params: Serialize + DeserializeOwned + JsonSchema + Send + 'static;
}

pub struct MethodDoc {
    pub name: &'static str,
    pub description: &'static str,
    pub errors: &'static [ErrorKind],
    pub params: serde_json::Value,
    pub result: serde_json::Value,
}

pub struct EventDoc {
    pub name: &'static str,
    pub description: &'static str,
    pub params: serde_json::Value,
}

/// Each method and event of the protocol, with its JSON Schema.
pub struct Catalog {
    pub methods: Vec<MethodDoc>,
    pub events: Vec<EventDoc>,
    generator: schemars::SchemaGenerator,
}

impl Default for Catalog {
    fn default() -> Catalog {
        Catalog {
            methods: Vec::new(),
            events: Vec::new(),
            generator: schemars::generate::SchemaSettings::draft2020_12().into_generator(),
        }
    }
}

impl Catalog {
    pub fn method<M: Method>(&mut self) -> &mut Catalog {
        assert!(
            !self.methods.iter().any(|m| m.name == M::NAME),
            "method {} is in the catalog two times",
            M::NAME
        );
        let params = self.generator.subschema_for::<M::Params>().to_value();
        let result = self.generator.subschema_for::<M::Result>().to_value();
        self.methods.push(MethodDoc {
            name: M::NAME,
            description: M::DESCRIPTION,
            errors: M::ERRORS,
            params,
            result,
        });
        self
    }

    pub fn event<E: Event>(&mut self) -> &mut Catalog {
        assert!(
            !self.events.iter().any(|e| e.name == E::NAME),
            "event {} is in the catalog two times",
            E::NAME
        );
        let params = self.generator.subschema_for::<E::Params>().to_value();
        self.events.push(EventDoc {
            name: E::NAME,
            description: E::DESCRIPTION,
            params,
        });
        self
    }

    /// The shared type definitions, by name. This empties the generator.
    pub(crate) fn take_definitions(&mut self) -> serde_json::Map<String, serde_json::Value> {
        self.generator.take_definitions(true)
    }
}

/// The whole protocol. To add a namespace, add one `register` line here.
pub fn catalog() -> Catalog {
    let mut c = Catalog::default();
    crate::core_api::register(&mut c);
    crate::terminal_api::register(&mut c);
    crate::voice_api::register(&mut c);
    crate::desk_api::register(&mut c);
    crate::agency_api::register(&mut c);
    crate::marks_api::register(&mut c);
    crate::knowledge_api::register(&mut c);
    crate::roles_api::register(&mut c);
    crate::meeting_api::register(&mut c);
    crate::call_api::register(&mut c);
    crate::models_api::register(&mut c);
    crate::vocabulary_api::register(&mut c);
    crate::hosts_api::register(&mut c);
    crate::story_api::register(&mut c);
    crate::ops_api::register(&mut c);
    crate::clear_api::register(&mut c);
    crate::reflect_api::register(&mut c);
    crate::git_api::register(&mut c);
    crate::ticket_api::register(&mut c);
    crate::events::register(&mut c);
    c
}
