use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufWriter, Write};

use inflector::Inflector;

use self::error_types::{GenerateErrorTypes, JsonErrorTypes, RestJsonErrorTypes, XmlErrorTypes};
use self::json::JsonGenerator;
use self::query::QueryGenerator;
use self::rest_json::RestJsonGenerator;
use self::rest_xml::RestXmlGenerator;
use self::tests::generate_tests;
use self::type_filter::filter_types;
use crate::botocore::{Member, Shape, ShapeType};
use crate::util;
use crate::Service;

mod error_types;
mod json;
mod query;
mod rest_json;
mod rest_request_generator;
mod rest_response_parser;
mod rest_xml;
pub mod tests;
mod type_filter;
mod xml_payload_parser;

type FileWriter = BufWriter<File>;
type IoResult = ::std::io::Result<()>;

/// Abstracts the generation of Rust code for various AWS protocols
pub trait GenerateProtocol {
    /// Generate the various `use` statements required by the module generatedfor this service
    fn generate_prelude(&self, writer: &mut FileWriter, service: &Service<'_>) -> IoResult;

    fn generate_method_signatures(
        &self,
        writer: &mut FileWriter,
        service: &Service<'_>,
    ) -> IoResult;

    /// Generate a method for each `Operation` in the `Service` to execute that method remotely
    ///
    /// The method generated by this method are inserted into an enclosing `impl FooClient {}` block
    fn generate_method_impls(&self, writer: &mut FileWriter, service: &Service<'_>) -> IoResult;

    /// If possible, return the trait that can be derived to serialize types
    fn serialize_trait(&self) -> Option<&'static str> {
        None
    }

    /// If possible, return the trait that can be derived to deserialize types
    fn deserialize_trait(&self) -> Option<&'static str> {
        None
    }

    /// If necessary, generate a serializer for the specified type
    /// This should be necessary only if `serialize_trait` returns `None`
    fn generate_serializer(
        &self,
        _name: &str,
        _shape: &Shape,
        _service: &Service<'_>,
    ) -> Option<String> {
        None
    }

    /// If necessary, generate a deserializer for the specified type
    /// This should be necessary only if `deserialize_trait` returns `None`
    fn generate_deserializer(
        &self,
        _name: &str,
        _shape: &Shape,
        _service: &Service<'_>,
    ) -> Option<String> {
        None
    }

    /// Return the type used by this protocol for timestamps
    fn timestamp_type(&self) -> &'static str;
}

pub fn generate_source(service: &Service<'_>, writer: &mut FileWriter) -> IoResult {
    // EC2 service protocol is similar to query but not the same.  Rusoto is able to generate Rust code
    // from the service definition through the same QueryGenerator, but botocore uses a special class.
    // See https://github.com/boto/botocore/blob/dff99fdf2666accf6b448aef7f03fe3d66dd38fa/botocore/serialize.py#L259-L266 .
    match service.protocol() {
        "json" => generate(writer, service, JsonGenerator, JsonErrorTypes),
        "query" | "ec2" => generate(writer, service, QueryGenerator, XmlErrorTypes),
        "rest-json" => generate(writer, service, RestJsonGenerator, RestJsonErrorTypes),
        "rest-xml" => generate(writer, service, RestXmlGenerator, XmlErrorTypes),
        protocol => panic!("Unknown protocol {}", protocol),
    }
}

/// Translate a botocore field name to something rust-idiomatic and
/// escape reserved words with an underscore
pub fn generate_field_name(member_name: &str) -> String {
    let name = member_name.to_snake_case();
    if name == "return" || name == "type" || name == "match" {
        name + "_"
    } else {
        name
    }
}

/// The quick brown fox jumps over the lazy dog
fn generate<P, E>(
    writer: &mut FileWriter,
    service: &Service<'_>,
    protocol_generator: P,
    error_type_generator: E,
) -> IoResult
where
    P: GenerateProtocol,
    E: GenerateErrorTypes,
{
    // TODO: `use futures::future;` isn't used by all crates: only include it when needed
    writeln!(
        writer,
        "
        // =================================================================
        //
        //                           * WARNING *
        //
        //                    This file is generated!
        //
        //  Changes made to this file will be overwritten. If changes are
        //  required to the generated code, the service_crategen project
        //  must be updated to generate the changes.
        //
        // =================================================================
        #![allow(warnings)]

        use std::error::Error;
        use std::fmt;
        use futures::future;
        use futures::Future;
        use rusoto_core::request::{{BufferedHttpResponse, DispatchSignedRequest}};
        use rusoto_core::region;
        use rusoto_core::credential::ProvideAwsCredentials;
        use rusoto_core::{{Client, RusotoFuture, RusotoError}};
    "
    )?;

    protocol_generator.generate_prelude(writer, service)?;
    generate_types(writer, service, &protocol_generator)?;
    error_type_generator.generate_error_types(writer, service)?;
    generate_client(writer, service, &protocol_generator)?;
    generate_tests(writer, service)?;

    Ok(())
}

fn generate_client<P>(
    writer: &mut FileWriter,
    service: &Service<'_>,
    protocol_generator: &P,
) -> IoResult
where
    P: GenerateProtocol,
{
    // If the struct name is changed, the links in each service documentation should change.
    // See https://github.com/rusoto/rusoto/issues/519
    writeln!(writer,
             "/// Trait representing the capabilities of the {service_name} API. {service_name} clients implement this trait.
        pub trait {trait_name} {{
        ",
             trait_name = service.service_type_name(),
             service_name = service.name())?;

    protocol_generator.generate_method_signatures(writer, service)?;

    writeln!(writer, "}}")?;

    writeln!(writer,
        "/// A client for the {service_name} API.
        #[derive(Clone)]
        pub struct {type_name} {{
            client: Client,
            region: region::Region,
        }}

        impl {type_name} {{
            /// Creates a client backed by the default tokio event loop.
            ///
            /// The client will use the default credentials provider and tls client.
            pub fn new(region: region::Region) -> {type_name} {{
                Self::new_with_client(Client::shared(), region)
            }}

            pub fn new_with<P, D>(request_dispatcher: D, credentials_provider: P, region: region::Region) -> {type_name}
                where P: ProvideAwsCredentials + Send + Sync + 'static,
                      P::Future: Send,
                      D: DispatchSignedRequest + Send + Sync + 'static,
                      D::Future: Send
            {{
                Self::new_with_client(Client::new_with(credentials_provider, request_dispatcher), region)
            }}

            pub fn new_with_client(client: Client, region: region::Region) -> {type_name}
            {{
                {type_name} {{
                    client,
                    region
                }}
            }}
        }}

        impl fmt::Debug for {type_name} {{
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{
                f.debug_struct(\"{type_name}\")
                    .field(\"region\", &self.region)
                    .finish()
            }}
        }}

        impl {trait_name} for {type_name} {{
        ",
        service_name = service.name(),
        type_name = service.client_type_name(),
        trait_name = service.service_type_name(),
    )?;
    protocol_generator.generate_method_impls(writer, service)?;
    writeln!(writer, "}}")
}

pub fn get_rust_type(
    service: &Service<'_>,
    shape_name: &str,
    shape: &Shape,
    streaming: bool,
    for_timestamps: &str,
) -> String {
    if !streaming {
        match shape.shape_type {
            ShapeType::Blob => "bytes::Bytes".into(),
            ShapeType::Boolean => "bool".into(),
            ShapeType::Double => "f64".into(),
            ShapeType::Float => "f32".into(),
            ShapeType::Integer | ShapeType::Long => "i64".into(),
            ShapeType::String => "String".into(),
            ShapeType::Timestamp => for_timestamps.into(),
            ShapeType::List => format!(
                "Vec<{}>",
                get_rust_type(
                    service,
                    shape.member_type(),
                    service.get_shape(shape.member_type()).unwrap(),
                    false,
                    for_timestamps
                )
            ),
            ShapeType::Map => format!(
                "::std::collections::HashMap<{}, {}>",
                get_rust_type(
                    service,
                    shape.key_type(),
                    service.get_shape(shape.key_type()).unwrap(),
                    false,
                    for_timestamps
                ),
                get_rust_type(
                    service,
                    shape.value_type(),
                    service.get_shape(shape.value_type()).unwrap(),
                    false,
                    for_timestamps
                ),
            ),
            ShapeType::Structure => mutate_type_name(service, shape_name),
        }
    } else {
        mutate_type_name_for_streaming(shape_name)
    }
}

fn streaming_members<'a>(shape: &'a Shape) -> Box<dyn Iterator<Item = &'a Member> + 'a> {
    let it = shape
        .members
        .as_ref()
        .into_iter()
        .flat_map(std::collections::BTreeMap::values)
        .filter(|&member| member.streaming());
    Box::new(it)
}

fn is_streaming_shape(service: &Service<'_>, name: &str) -> bool {
    service
        .shapes()
        .iter()
        .any(|(_, shape)| streaming_members(shape).any(|member| member.shape == name))
}

// do any type name mutation for shapes needed to avoid collisions with Rust types and Error enum types
fn mutate_type_name(service: &Service<'_>, type_name: &str) -> String {
    let capitalized = util::capitalize_first(type_name.to_owned());

    // some cloudfront types have underscoare that anger the lint checker
    let without_underscores = capitalized.replace("_", "");

    match &without_underscores[..] {
        // Some services have an 'Error' shape that collides with Rust's Error trait
        "Error" => format!("{}Error", service.service_type_name()),

        // EC2 has a CancelSpotFleetRequestsError struct, avoid collision with our error enum
        "CancelSpotFleetRequests" => "EC2CancelSpotFleetRequests".to_owned(),

        // Glue has a BatchStopJobRunError struct, avoid collision with our error enum
        "BatchStopJobRun" => "GlueBatchStopJobRun".to_owned(),

        // RDS has a conveniently named "Option" type
        "Option" => "RDSOption".to_owned(),

        // Discovery has an BatchDeleteImportDataError struct, avoid collision with our error enum
        "BatchDeleteImportDataError" => "DiscoveryBatchDeleteImportDataError".to_owned(),

        // EC2 has an CreateFleetError struct, avoid collision with our error enum
        "CreateFleetError" => "EC2CreateFleetError".to_owned(),

        // codecommit has a BatchDescribeMergeConflictsError, avoid collision with our error enum
        "BatchDescribeMergeConflictsError" => "CodeCommitBatchDescribeMergeConflictsError".to_owned(),

        // codecommit has a BatchGetCommitsError, avoid collision with our error enum
        "BatchGetCommitsError" => "CodeCommitBatchGetCommitsError".to_owned(),

        // otherwise make sure it's rust-idiomatic and capitalized
        _ => without_underscores,
    }
}

// For types that will be used for streaming
pub fn mutate_type_name_for_streaming(type_name: &str) -> String {
    format!("Streaming{}", type_name)
}

fn find_shapes_to_generate(service: &Service<'_>) -> BTreeSet<String> {
    let mut shapes_to_generate = BTreeSet::<String>::new();

    let mut visitor = |shape_name: &str, _shape: &Shape| {
        shapes_to_generate.insert(shape_name.to_owned())
    };

    for operation in service.operations().values() {
        if let Some(ref input) = operation.input {
            service.visit_shapes(&input.shape, &mut visitor);
        }
        if let Some(ref output) = operation.output {
            service.visit_shapes(&output.shape, &mut visitor);
        }
        if let Some(ref errors) = operation.errors {
            for error in errors {
                service.visit_shapes(&error.shape, &mut visitor);
            }
        }
    }
    shapes_to_generate
}

fn generate_types<P>(
    writer: &mut FileWriter,
    service: &Service<'_>,
    protocol_generator: &P,
) -> IoResult
where
    P: GenerateProtocol,
{
    let (serialized_types, deserialized_types) = filter_types(service);

    for name in find_shapes_to_generate(service).iter() {
        let shape = service.get_shape(name).unwrap();

        // We generate enums for error types, so no need to create model objects for them
        // Kinesis is a special case in that some operations return
        // responses whose struct fields refer to expecific error shapes
        if shape.exception() && service.name() != "Kinesis" {
            continue;
        }

        let type_name = mutate_type_name(service, name);

        let streaming = is_streaming_shape(service, name);
        let deserialized = deserialized_types.contains(&type_name);
        let serialized = serialized_types.contains(&type_name);

        if shape.shape_type == ShapeType::Structure {
            // If botocore includes documentation, clean it up a bit and use it
            if let Some(ref docs) = shape.documentation {
                writeln!(writer, "{}", crate::doco::Item(docs))?;
            }

            // generate a rust type for the shape
            if type_name != "String" {
                let generated = generate_struct(
                    service,
                    &type_name,
                    shape,
                    streaming,
                    serialized,
                    deserialized,
                    protocol_generator,
                );
                writeln!(writer, "{}", generated)?;
            }
        }

        if streaming {
            // Add a second type for streaming blobs, which are the only streaming type we can have
            writeln!(
                writer,
                "pub type {} = ::rusoto_core::ByteStream;",
                mutate_type_name_for_streaming(&type_name)
            )?;
        }

        if deserialized {
            if let Some(deserializer) =
                protocol_generator.generate_deserializer(&type_name, shape, service)
            {
                assert!(protocol_generator.deserialize_trait().is_none());
                writeln!(writer, "{}", deserializer)?;
            }
        }

        if serialized {
            if let Some(serializer) =
                protocol_generator.generate_serializer(&type_name, shape, service)
            {
                assert!(protocol_generator.serialize_trait().is_none());
                writeln!(writer, "{}", serializer)?;
            }
        }
    }
    Ok(())
}

fn generate_struct<P>(
    service: &Service<'_>,
    name: &str,
    shape: &Shape,
    streaming: bool,
    serialized: bool,
    deserialized: bool,
    protocol_generator: &P,
) -> String
where
    P: GenerateProtocol,
{
    let mut derived = vec!["Default", "Debug"];

    // Streaming is implemented with Box<Stream<...>>, so we can't derive Clone nor PartialEq.
    // This affects both the streaming struct itself, and structs which contain it.
    if !streaming && streaming_members(shape).next().is_none() {
        derived.push("Clone");
        derived.push("PartialEq");
    }

    if serialized {
        if let Some(serialize_trait) = protocol_generator.serialize_trait() {
            derived.push(serialize_trait);
        }
    }

    if deserialized {
        if let Some(deserialize_trait) = protocol_generator.deserialize_trait() {
            derived.push(deserialize_trait);
        }
    }

    let attributes = format!("#[derive({})]", derived.join(","));
    let test_attributes = if derived.iter().any(|&x| x == "Deserialize")
        && !derived.iter().any(|&x| x == "Serialize")
    {
        "\n#[cfg_attr(any(test, feature = \"serialize_structs\"), derive(Serialize))]"
    } else {
        ""
    };

    if shape.members.is_none() || shape.members.as_ref().unwrap().is_empty() {
        format!(
            "{attributes}{test_attributes}
            pub struct {name} {{}}
            ",
            attributes = attributes,
            test_attributes = test_attributes,
            name = name,
        )
    } else {
        // Serde attributes are only needed if deriving the Serialize or Deserialize trait
        let need_serde_attrs = derived
            .iter()
            .any(|&x| x == "Serialize" || x == "Deserialize");
        format!(
            "{attributes}{test_attributes}
            pub struct {name} {{
                {struct_fields}
            }}
            ",
            attributes = attributes,
            test_attributes = test_attributes,
            name = name,
            struct_fields =
                generate_struct_fields(service, shape, name, need_serde_attrs, protocol_generator),
        )
    }
}

fn generate_struct_fields<P: GenerateProtocol>(
    service: &Service<'_>,
    shape: &Shape,
    shape_name: &str,
    serde_attrs: bool,
    protocol_generator: &P,
) -> String {
    shape.members.as_ref().unwrap().iter().filter_map(|(member_name, member)| {
        if member.deprecated == Some(true) {
            return None;
        }

        let mut lines: Vec<String> = Vec::new();

        if let Some(ref docs) = member.documentation {
            lines.push(crate::doco::Item(docs).to_string());
        }

        if serde_attrs {
            lines.push(format!("#[serde(rename=\"{}\")]", member_name));

            if let Some(member_shape) = service.shape_for_member(member) {
                if member_shape.shape_type == ShapeType::Blob {
                    lines.push(
                        "#[serde(
                            deserialize_with=\"::rusoto_core::serialization::SerdeBlob::deserialize_blob\",
                            serialize_with=\"::rusoto_core::serialization::SerdeBlob::serialize_blob\",
                            default,
                        )]".to_owned()
                    );
                } else if member_shape.shape_type == ShapeType::List {
                    if let Some(ref list_element_member) = member_shape.member {
                        if let Some(list_element_shape_type) = service.shape_type_for_member(list_element_member) {
                            if list_element_shape_type == ShapeType::Blob {
                                lines.push(
                                    "#[serde(
                                        deserialize_with=\"::rusoto_core::serialization::SerdeBlobList::deserialize_blob_list\",
                                        serialize_with=\"::rusoto_core::serialization::SerdeBlobList::serialize_blob_list\",
                                        default,
                                    )]".to_owned()
                                );
                            }
                        }
                    }
                }
            }

            if !shape.required(member_name) {
                lines.push("#[serde(skip_serializing_if=\"Option::is_none\")]".to_owned());
            }
        }

        let member_shape = service.shape_for_member(member).unwrap();
        let rs_type = get_rust_type(service,
                                    &member.shape,
                                    member_shape,
                                    member.streaming(),
                                    protocol_generator.timestamp_type());
        let name = generate_field_name(member_name);

        // For structs that can contain another of themselves, we need to box them.
        if shape_name == rs_type {
            if shape.required(member_name) {
                lines.push(format!("pub {}: Box<{}>,", name, rs_type))
            } else if name == "type" {
                lines.push(format!("pub aws_{}: Box<Option<{}>>,", name,rs_type))
            } else {
                lines.push(format!("pub {}: Box<Option<{}>>,", name, rs_type))
            }
        } else {
            // In the official documentation the fields revision_change_id and created are required
            // but when looking at the responses from aws those are not always set.
            // See https://github.com/rusoto/rusoto/issues/1419 for more information
            if service.name() == "CodePipeline" && shape_name == "ActionRevision" && name == "revision_change_id" || name == "created" {
                lines.push(format!("pub {}: Option<{}>,", name, rs_type))
            // In pratice, Lex can return null values for slots that are not filled. The documentation
            // does not mention that the slot values themselves can be null.
            } else if service.name() == "Amazon Lex Runtime Service"  && shape_name == "PostTextResponse" && name == "slots"{
                lines.push(format!("pub {}: Option<::std::collections::HashMap<String, Option<String>>>,", name))
            } else if shape.required(member_name) {
                lines.push(format!("pub {}: {},", name, rs_type))
            } else if name == "type" {
                lines.push(format!("pub aws_{}: Option<{}>,", name,rs_type))
            } else {
                lines.push(format!("pub {}: Option<{}>,", name, rs_type))
            }
        }

        Some(lines.join("\n"))
    }).collect::<Vec<String>>().join("\n")
}

fn error_type_name(service: &Service<'_>, name: &str) -> String {
    let type_name = mutate_type_name(service, name);
    format!("{}Error", type_name)
}
