//! The adapter uses the generic ERP input contract, not a game's field names.
use std::cell::RefCell;
use std::rc::Rc;

use orr_mcp::{Bridge, ErpCall, McpServer, ToolGroups};
use orr_remote::{ClientError, RpcError, METHOD_NOT_FOUND};
use serde_json::{json, Value as J};

type Calls = Rc<RefCell<Vec<(String, J)>>>;

struct Fake {
    calls: Calls,
    result: J,
    unsupported: bool,
}

impl ErpCall for Fake {
    fn call(&mut self, method: &str, params: J) -> Result<J, ClientError> {
        self.calls.borrow_mut().push((method.to_string(), params));
        if self.unsupported {
            return Err(ClientError::Rpc(RpcError::new(METHOD_NOT_FOUND, "input_unavailable", "structured input is unavailable")));
        }
        Ok(self.result.clone())
    }
}

fn server(result: J, unsupported: bool) -> (McpServer, Calls) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = calls.clone();
    let bridge = Bridge::new("fake", Box::new(move || Ok(Box::new(Fake { calls: log.clone(), result: result.clone(), unsupported }))));
    let mut server = McpServer::new(bridge, ToolGroups::ALL);
    let initialized = server.handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": orr_mcp::PROTOCOL_VERSION, "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}})).unwrap();
    assert!(initialized.get("result").is_some(), "{initialized}");
    (server, calls)
}

fn call(server: &mut McpServer, name: &str, arguments: J) -> J {
    server.handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": name, "arguments": arguments}})).unwrap()["result"].clone()
}

#[test]
fn input_survives_the_mcp_json_parser_without_decimal_rounding() {
    let (mut server, calls) = server(json!({"ok": true}), false);
    let response = server.handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"sim_input","arguments":{"player":3,"value":{"throttle":0.1234567890123456789012345,"actions":["jump"]}}}}"#).unwrap();
    let response: J = serde_json::from_str(&response).unwrap();
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(response["result"]["structuredContent"], json!({"ok": true}));
    let log = calls.borrow();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, "sim.input_value");
    assert_eq!(log[0].1.to_string(), r#"{"player":3,"value":{"throttle":0.1234567890123456789012345,"actions":["jump"]}}"#);
}

#[test]
fn input_schema_returns_the_complete_descriptor() {
    let descriptor = json!({"schema": {"type": "object", "properties": {"throttle": {"type": "number", "minimum": -1, "maximum": 1}}}, "value_format": "exact decimals"});
    let (mut server, calls) = server(descriptor.clone(), false);
    let result = call(&mut server, "get_schema", json!({"input": true}));
    assert_eq!(result["isError"], false);
    assert_eq!(result["structuredContent"], descriptor);
    assert_eq!(calls.borrow().as_slice(), &[("registry.input".into(), J::Null)]);
}

#[test]
fn malformed_input_and_schema_conflicts_do_not_contact_the_host() {
    let (mut server, calls) = server(J::Null, false);
    for args in [json!({}), json!({"player": 0}), json!({"player": -1, "value": {}}), json!({"player": 0.5, "value": {}}), json!({"player": 0, "value": []}), json!({"player": 0, "value": null})] {
        assert_eq!(call(&mut server, "sim_input", args)["isError"], true);
    }
    for args in [json!({"input": true, "type": "Body"}), json!({"input": true, "list_types": true}), json!({"input": "true"})] {
        assert_eq!(call(&mut server, "get_schema", args)["isError"], true);
    }
    assert!(calls.borrow().is_empty());
}

#[test]
fn unavailable_input_is_reported_without_a_raw_input_fallback() {
    let (mut server, calls) = server(J::Null, true);
    for (tool, args) in [("get_schema", json!({"input": true})), ("sim_input", json!({"player": 0, "value": {"throttle": 1}}))] {
        let result = call(&mut server, tool, args);
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"].as_str().unwrap().contains("input_unavailable"));
    }
    assert_eq!(calls.borrow().iter().map(|call| call.0.as_str()).collect::<Vec<_>>(), ["registry.input", "sim.input_value"]);
}

#[test]
fn input_tool_is_in_sim_group_with_the_expected_capability_and_schema() {
    let tool = orr_mcp::tools::find("sim_input").unwrap();
    assert_eq!(tool.group, "sim");
    assert_eq!(tool.needs, "sim_control");
    assert!(!tool.read_only);
    let schema = tool.input_schema();
    assert_eq!(schema["required"], json!(["player", "value"]));
    assert_eq!(schema["properties"]["value"]["type"], "object");
    assert!(orr_mcp::tools::defs(ToolGroups::parse("sim").unwrap()).iter().any(|t| t.name == "sim_input"));
    assert!(!orr_mcp::tools::defs(ToolGroups::parse("scene").unwrap()).iter().any(|t| t.name == "sim_input"));
}

#[test]
fn generated_guide_includes_input_only_when_the_host_advertises_it() {
    let mut discovery = json!({"engine": {"name": "orrery", "version": "test", "game": "GenericGame", "metrics": []}});
    let types = json!({"types": []});
    let schema = json!({"$defs": {}});
    let ordinary = orr_mcp::agents_md::render(&discovery, &types, &schema, ToolGroups::ALL);
    assert!(!ordinary.contains("## Structured player input"));
    discovery["engine"]["input"] = json!({"schema": {"type": "object", "properties": {"throttle": {"type": "number"}}}, "value_format": "exact test decimal format"});
    let guide = orr_mcp::agents_md::render(&discovery, &types, &schema, ToolGroups::ALL);
    for expected in ["## Structured player input", "orr schema --input", "sim_input", "throttle", "exact test decimal format", "--replay-out", "--force", "Existing paths are refused before stopping play", "recording_matches", "The host does not save the file"] {
        assert!(guide.contains(expected), "guide must explain {expected}");
    }
    assert!(!guide.contains("axis_x"), "guide must discover game-specific fields rather than assume Arena");
}

#[test]
fn arena_guide_uses_the_arena_host_without_claiming_editor_support() {
    let discovery = json!({"engine": {"name": "orrery", "version": "test", "game": "Arena", "metrics": []}});
    let guide = orr_mcp::agents_md::render(&discovery, &json!({"types": []}), &json!({"$defs": {}}), ToolGroups::ALL);
    assert!(guide.contains("orr_remote_host --game arena --dev-no-auth"));
    assert!(guide.contains("orr_remote_host --game arena --token"));
    assert!(!guide.contains("orr_editor") && !guide.contains("Agent tab") && !guide.contains("in the editor"));
    assert!(!guide.contains("orr_remote_host --dev-no-auth"), "the default host would run PhysGame");
}
