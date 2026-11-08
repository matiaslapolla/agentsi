# The core protocol

`core.openrpc.json` is the OpenRPC schema of the core protocol: each method with its params, its result and its errors in JSON Schema. `agentsi-proto` makes the schema from the Rust types. A running core returns the same document for `rpc.discover`.

**Status:** protocol 1, unstable until M2. During M1 a method or a field can change without notice. At M2 the schema becomes a stable public contract. Spec: `docs/specs/agentsi-desktop-protocol.md`. The format copies Herdr's `herdr-core` (Apache-2.0).

## Update the schema

A test in `agentsi-proto` fails when `core.openrpc.json` is not the same as the schema of the Rust types. After a change of a type, write the file again:

```sh
AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema
```

Then commit the new file with the change of the type.

## Socket

- Linux: `$XDG_RUNTIME_DIR/agentsi/core.sock`. `AGENTSI_CORE_SOCKET` overrides it.
- The core makes the default folder with mode 0700 and the socket with mode 0600.
- The core closes a connection from another user before it reads from it.
- `core.lock` next to the socket holds the pid of the core. A second core finds the lock taken and stops with exit code 3.
- `agentsi-core connect` joins stdin and stdout to the socket. Thus `ssh -T <host> agentsi-core connect` reaches the core of an SSH host. If no core runs, `connect` starts one.

## Line mode

A connection starts in line mode: JSON-RPC 2.0, one message on each line. `socat` is a sufficient client.

```
→ {"jsonrpc":"2.0","id":1,"method":"core.hello","params":{}}
← {"jsonrpc":"2.0","id":1,"result":{"features":["core","framed","rpc"],"protocol":1,"version":"0.0.0"}}
```

The protocol takes params by name only. It has no batches. A line takes at most 16 MiB, without the newline. A request without an id gets no reply. In line mode the core answers the requests of one connection in sequence.

## Framed mode

`core.hello` with `{"framed": true}` changes the connection to framed mode after the reply. Then each message is a frame:

```
frame = [channel u32 LE][kind u8][len u32 LE][payload ≤ 16 MiB]
```

| Kind | Name | Payload |
|---|---|---|
| 0 | DATA | Channel 0: one JSON-RPC message. Other channels: stream bytes. |
| 1 | CREDIT | u32 LE: the bytes that the receiver consumed. |
| 2 | CLOSE | The channel ends. From the core, a JSON reason. |
| 3 | RESIZE | u16 LE cols, u16 LE rows, from a pane to the core. |
| 4 | FOCUS | 1 or 0, from a pane to the core. |
| 5 | AUTO | Reply bytes that the screen of a pane made, not typed keys. |

- In framed mode the core answers the requests on channel 0 at the same time. Thus a slow method does not stop the other requests.
- The requests in flight on one connection take at most 32 MiB. If a client does not read its replies, the core stops to read its requests.
- A receiver ignores a frame kind that it does not know.
- A frame with a payload of more than 16 MiB closes the connection.
- The core sends at most 1 MiB on a stream channel before the client returns a CREDIT. The terminal streams come with U7.

## Events

`core.subscribe {"topics": [...]}` adds topics to the connection. The topics are `terminal`, `data`, `meeting`, `voice` and `dictation`. Then the core sends each event of these topics as a notification on channel 0. The `x-notifications` list of the schema gives the params of each event.

```
{"jsonrpc":"2.0","method":"terminal.state","params":{"id":"t1","state":"blocked","kind":"permission"}}
```

The core keeps 1024 events of the topics of each connection. If a connection reads too slowly, the core drops its new events. Then it sends `core.lagged {"missed": <n>}` where the events are missing. Then the client must read the state again. Events of other topics do not count. Each subscriber gets the `core` events, with no topic.

## Errors

Each error is a JSON-RPC error with a stable `data.kind` and a `data.hint` for a person:

```
{"code":-32601,"message":"unknown method \"x\"","data":{"kind":"not_found","hint":"Call rpc.discover for the list of methods."}}
```

The kinds are `not_found`, `invalid`, `conflict`, `busy`, `unavailable` and `internal`. A parse error, a bad request and bad params have the kind `invalid`. An unknown method has the kind `not_found`.

## Compatibility

From M2, `released.openrpc.json` holds the schema of the last release. A test in `agentsi-proto` fails on a change that breaks a client of that schema:

- a method, a parameter, a result field or an event that goes away,
- a new required parameter,
- a parameter that accepts less,
- a result field that changes its type or can be missing now.

During M1 the file does not exist, and that test passes. A break that the project wants gets protocol number 2, on purpose.
