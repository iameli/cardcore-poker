/**
 * Cross-language wire-compatibility proof, Cardcore side.
 *
 * The fixtures below are the same base64url payloads asserted by the atbloons
 * Kotlin test `CardCoreWalletHandoffInteropTest`. This test proves the Cardcore
 * client decodes a wallet-produced receipt and produces the exact intent the
 * wallet decodes. Both repositories share one wire contract.
 *
 * Run with: node --test packages/web/src/lib/atbloons/interop.test.js
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  COMMAND_KIND,
  bytesToBase64Url,
  decodeIntent,
  decodeReceipt,
  encodeIntent,
} from "./handoff.js";

const SCOPE = {
  networkId: "atbloons-cardcore-testnet",
  protocolVersion: "v3",
  genesisHash: "a".repeat(64),
};
const STATE = bytesToBase64Url(new Uint8Array(16).map((_, i) => i));
const CID = "bafyreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku";

// Exact base64url string asserted by CardCoreWalletHandoffInteropTest (Kotlin).
const KOTLIN_ENCODES_PROPOSE =
  "eyJ2ZXJzaW9uIjoiY2FyZGNvcmUtcG9rZXItY29udHJhY3QtdjEiLCJraW5kIjoiUFJPUE9TRSIsInNjb3BlIjp7Im5ldHdvcmtJZCI6ImF0Ymxvb25zLWNhcmRjb3JlLXRlc3RuZXQiLCJwcm90b2NvbFZlcnNpb24iOiJ2MyIsImdlbmVzaXNIYXNoIjoiYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYSJ9LCJyZXR1cm5VcmwiOiJodHRwczovL2dhbWUuZXhhbXBsZS9jYWxsYmFjayIsInN0YXRlIjoiQUFFQ0F3UUZCZ2NJQ1FvTERBME9EdyIsInRhYmxlIjp7InVyaSI6ImF0Oi8vZGlkOnBsYzphbGljZS9yZS5jYXJkY28ucG9rZXIudGFibGUvdGFibGUiLCJjaWQiOiJiYWZ5cmVpaGR3ZGNlZmdoNGRxa2p2Njd1emNtdzdvamVlNnhlZHpkZXRvanV6amV2dGVueHF1dnlrdSJ9LCJzb3Vsc1BlckNoaXAiOiI5MjIzMzcyMDM2ODU0Nzc1ODA3In0";

// Exact receipt base64url the wallet returns and the Kotlin test emits.
const WALLET_FUND_RECEIPT =
  "eyJ2ZXJzaW9uIjoiY2FyZGNvcmUtcG9rZXItY29udHJhY3QtdjEiLCJzdGF0ZSI6IkFBRUNBd1FGQmdjSUNRb0xEQTBPRHciLCJraW5kIjoiRlVORCIsInN0YXR1cyI6ImNvbXBsZXRlIiwicmVjb3JkcyI6W3siY29sbGVjdGlvbiI6InRlY2gubGVub29ieTA5LmF0Ymxvb25zLmNvbnRyYWN0RnVuZGluZyIsInJlZmVyZW5jZSI6eyJ1cmkiOiJhdDovL2RpZDpwbGM6Ym9iL3RlY2gubGVub29ieTA5LmF0Ymxvb25zLmNvbnRyYWN0RnVuZGluZy9mdW5kaW5nIiwiY2lkIjoiYmFmeXJlaWhkd2RjZWZnaDRkcWtqdjY3dXpjbXc3b2plZTZ4ZWR6ZGV0b2p1empldnRlbnhxdXZ5a3UifX1dLCJjb250aW51YXRpb24iOiJBQUVDQXdRRkJnY0lDUW9MREEwT0R4QVJFaE1VRlJZWEdCa2FHeHdkSGg4In0";

test("Cardcore reproduces the intent bytes the wallet decodes", () => {
  const encoded = encodeIntent({
    kind: COMMAND_KIND.PROPOSE,
    scope: SCOPE,
    returnUrl: "https://game.example/callback",
    state: STATE,
    table: { uri: "at://did:plc:alice/re.cardco.poker.table/table", cid: CID },
    soulsPerChip: "9223372036854775807",
  });
  assert.equal(encoded, KOTLIN_ENCODES_PROPOSE);
  assert.equal(decodeIntent(KOTLIN_ENCODES_PROPOSE).kind, "PROPOSE");
});

test("Cardcore decodes the wallet FUND receipt", () => {
  const receipt = decodeReceipt(WALLET_FUND_RECEIPT);
  assert.equal(receipt.kind, "FUND");
  assert.equal(receipt.status, "complete");
  assert.equal(receipt.records.length, 1);
  assert.equal(receipt.records[0].collection, "tech.lenooby09.atbloons.contractFunding");
  assert.equal(receipt.continuation.length, 43);
});
