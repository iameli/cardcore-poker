/**
 * Contract discovery for the atbloons paid hand.
 *
 * A non-host seat needs the exact contract strong reference to fund. Rather
 * than paste it by hand, the seat reads the host's public repository: the host
 * (seat 0) publishes the `tech.lenooby09.atbloons.contract` record to its own
 * PDS, and that record strong-references the finalized table. The seat lists
 * the host's contract records and selects the one that references this table.
 *
 * This uses only public AT records. It touches no wallet, OAuth token, or DPoP
 * key. The confidential wallet still re-verifies every reference before it
 * spends, so a wrong or forged pointer cannot move value.
 */

import { CONTRACT_COLLECTION } from "./handoff.js";

/**
 * Select the contract record that strong-references the given finalized table.
 * @param {Array<{uri:string, cid:string, value:object}>} records - listRecords items
 * @param {{uri:string, cid:string}} tableRef - the finalized table strong ref
 * @returns {{uri:string, cid:string}|null} the contract strong ref, or null
 */
export function selectContractForTable(records, tableRef) {
  if (!tableRef || !tableRef.uri || !tableRef.cid) return null;
  for (const rec of records || []) {
    const table = rec && rec.value && rec.value.table;
    if (table && table.uri === tableRef.uri && table.cid === tableRef.cid) {
      if (rec.uri && rec.cid) return { uri: rec.uri, cid: rec.cid };
    }
  }
  return null;
}

/**
 * Look up the contract strong reference for a table from the host's repo.
 * Pages through `com.atproto.repo.listRecords` and returns the first contract
 * that references the table, or null when none is found or the query fails.
 * @param {object} opts
 * @param {{get:Function}} opts.client - an @atcute-style client with `get`
 * @param {string} opts.hostDid - the host (seat 0) DID that owns the contract
 * @param {{uri:string, cid:string}} opts.tableRef - the finalized table strong ref
 * @returns {Promise<{uri:string, cid:string}|null>}
 */
export async function lookupContractForTable({ client, hostDid, tableRef }) {
  if (!client || typeof client.get !== "function") return null;
  if (!hostDid || !tableRef || !tableRef.uri) return null;
  let cursor;
  // A bound keeps a hostile or broken cursor from looping forever.
  for (let page = 0; page < 64; page++) {
    const params = { repo: hostDid, collection: CONTRACT_COLLECTION.CONTRACT, limit: 100 };
    if (cursor) params.cursor = cursor;
    let res;
    try {
      res = await client.get("com.atproto.repo.listRecords", { params });
    } catch {
      return null;
    }
    if (!res || !res.ok) return null;
    const records = (res.data && res.data.records) || [];
    const found = selectContractForTable(records, tableRef);
    if (found) return found;
    cursor = records.length ? res.data.cursor : undefined;
    if (!cursor) return null;
  }
  return null;
}
