import assert from "node:assert/strict";
import { test } from "node:test";
import { isLocalHost } from "./local-host.ts";

test("the loopback hostnames pass with or without a port", () => {
  for (const host of ["127.0.0.1", "127.0.0.1:3361", "localhost", "localhost:3364", "LOCALHOST:3361", "[::1]", "[::1]:3365"]) {
    assert.equal(isLocalHost(host), true, host);
  }
});

test("any other hostname and a missing Host fail", () => {
  for (const host of [null, "", "evil.example", "evil.example:3361", "127.0.0.1.evil.example", "localhost.evil.example", "localhost.", "127.0.0.2:3361", "0.0.0.0:3361", "::1", "[::1]x", "localhost:3361:1", "192.168.1.5:3361"]) {
    assert.equal(isLocalHost(host), false, String(host));
  }
});
