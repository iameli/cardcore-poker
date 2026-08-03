/**
 * Offline tests for atbloons contract discovery. No network: an injected client
 * returns fixed `listRecords` pages.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { selectContractForTable, lookupContractForTable } from "./contract-lookup.js";

const TABLE = {
  uri: "at://did:plc:host/re.cardco.poker.table/table",
  cid: "bafyreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku",
};
const CONTRACT = {
  uri: "at://did:plc:host/tech.lenooby09.atbloons.contract/abc",
  cid: "bafyreiaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
};

function contractRecord({ uri, cid, table }) {
  return { uri, cid, value: { $type: "tech.lenooby09.atbloons.contract", table } };
}

function clientReturning(pages) {
  let call = 0;
  return {
    calls: [],
    async get(_method, { params }) {
      this.calls.push(params);
      const page = pages[call] || { records: [] };
      call += 1;
      return { ok: true, data: page };
    },
  };
}

test("selectContractForTable matches on the exact table strong reference", () => {
  const records = [
    contractRecord({ uri: "at://x/c/1", cid: "cid1", table: { uri: TABLE.uri, cid: "other" } }),
    contractRecord({ uri: CONTRACT.uri, cid: CONTRACT.cid, table: TABLE }),
  ];
  assert.deepEqual(selectContractForTable(records, TABLE), {
    uri: CONTRACT.uri,
    cid: CONTRACT.cid,
  });
});

test("selectContractForTable returns null when nothing references the table", () => {
  const records = [
    contractRecord({ uri: "at://x/c/1", cid: "cid1", table: { uri: "at://z", cid: "zzz" } }),
  ];
  assert.equal(selectContractForTable(records, TABLE), null);
});

test("selectContractForTable ignores a table match with a different cid", () => {
  const records = [
    contractRecord({ uri: "at://x/c/1", cid: "cid1", table: { uri: TABLE.uri, cid: "wrong" } }),
  ];
  assert.equal(selectContractForTable(records, TABLE), null);
});

test("lookupContractForTable finds the contract on the first page", async () => {
  const client = clientReturning([
    { records: [contractRecord({ uri: CONTRACT.uri, cid: CONTRACT.cid, table: TABLE })] },
  ]);
  const found = await lookupContractForTable({
    client,
    hostDid: "did:plc:host",
    tableRef: TABLE,
  });
  assert.deepEqual(found, { uri: CONTRACT.uri, cid: CONTRACT.cid });
  assert.equal(client.calls[0].collection, "tech.lenooby09.atbloons.contract");
  assert.equal(client.calls[0].repo, "did:plc:host");
});

test("lookupContractForTable pages until it finds the contract", async () => {
  const client = clientReturning([
    {
      records: [
        contractRecord({ uri: "at://x/c/1", cid: "cid1", table: { uri: "at://z", cid: "z" } }),
      ],
      cursor: "c1",
    },
    { records: [contractRecord({ uri: CONTRACT.uri, cid: CONTRACT.cid, table: TABLE })] },
  ]);
  const found = await lookupContractForTable({
    client,
    hostDid: "did:plc:host",
    tableRef: TABLE,
  });
  assert.deepEqual(found, { uri: CONTRACT.uri, cid: CONTRACT.cid });
  assert.equal(client.calls.length, 2);
  assert.equal(client.calls[1].cursor, "c1");
});

test("lookupContractForTable returns null when the host has no matching contract", async () => {
  const client = clientReturning([{ records: [] }]);
  const found = await lookupContractForTable({
    client,
    hostDid: "did:plc:host",
    tableRef: TABLE,
  });
  assert.equal(found, null);
});

test("lookupContractForTable swallows a failed query and returns null", async () => {
  const client = {
    async get() {
      return { ok: false, status: 500 };
    },
  };
  const found = await lookupContractForTable({
    client,
    hostDid: "did:plc:host",
    tableRef: TABLE,
  });
  assert.equal(found, null);
});

test("lookupContractForTable returns null without a usable client", async () => {
  assert.equal(await lookupContractForTable({ client: null, hostDid: "d", tableRef: TABLE }), null);
});
