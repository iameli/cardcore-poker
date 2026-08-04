<script>
  import { onMount } from "svelte";
  import { resolveAtbloonsConfig, discoverAtbloonsConfig } from "../lib/atbloons/config.js";
  import { PaidHandController } from "../lib/atbloons/paid-hand.js";
  import { HandoffError } from "../lib/atbloons/handoff.js";

  // tableRef: { uri, cid } of the finalized re.cardco.poker.table record.
  // seat: our seat index in the table roster (0 is the host).
  // terminalActionRef: { uri, cid } of the completed hand's terminal action.
  // contractLookup: optional async () => { uri, cid } | null. A non-host seat
  //   uses it to discover the contract from the host repo instead of pasting.
  let { tableRef, seat = -1, terminalActionRef = null, contractLookup = null } = $props();

  const isHost = $derived(seat === 0);

  // A stable return origin+path so the wallet can pin the game origin and the
  // receipt returns to this same table view. No query, no fragment.
  const returnUrl =
    typeof window !== "undefined"
      ? `${window.location.origin}${window.location.pathname}`
      : "https://localhost/";

  let controller = null;
  // The panel shows as soon as a wallet is configured; the network tuple may
  // still be discovering from the node. `ready` gates the action controls.
  let walletConfigured = $state(false);
  let ready = $state(false);
  let snapshot = $state(null);
  let error = $state("");
  let receiptNote = $state("");

  // Host proposal input and shared contract, participant contract entry.
  let soulsPerChip = $state("1000");
  let contractUri = $state("");
  let contractCid = $state("");
  let terminalUri = $state("");
  let terminalCid = $state("");
  // A contract discovered from the host repo for a non-host seat, so funding is
  // one click. Null until discovery finds it; manual entry stays the fallback.
  let autoContract = $state(null);

  // The terminal action the game already published for this hand. When present,
  // settlement is one click: the host never types a strong reference by hand.
  const autoTerminal = $derived(
    terminalActionRef?.uri && terminalActionRef?.cid
      ? { uri: terminalActionRef.uri, cid: terminalActionRef.cid }
      : null,
  );

  function refresh() {
    snapshot = controller ? controller.snapshot() : null;
  }

  onMount(async () => {
    const base = resolveAtbloonsConfig();
    if (!base || !tableRef?.uri) return;
    // A wallet is configured: show the panel even while the tuple resolves.
    walletConfigured = true;
    let config = base;
    if (!base.scope) {
      // Managed-wallet discovery: read the exact network tuple from the node.
      config = await discoverAtbloonsConfig();
    }
    if (!config || !config.scope) {
      error = "Could not reach the atbloons node to confirm its network. Try again shortly.";
      return;
    }
    controller = new PaidHandController({
      walletBaseUrl: config.walletBaseUrl,
      scope: config.scope,
      returnUrl,
      storageKey: `atbloons.paid-hand.${tableRef.uri}`,
    });
    try {
      const receipt = controller.consumeReturn();
      if (receipt)
        receiptNote = `${receipt.kind} → ${receipt.status}${receipt.errorCode ? ` (${receipt.errorCode})` : ""}`;
    } catch (e) {
      error = describe(e);
    }
    ready = true;
    refresh();

    // A non-host seat discovers the contract from the host repo, so it funds
    // without pasting a strong reference. Failure leaves manual entry in place.
    if (!isHost && contractLookup && (!snapshot || snapshot.grant === "idle")) {
      try {
        const found = await contractLookup();
        if (found?.uri && found?.cid) {
          autoContract = { uri: found.uri, cid: found.cid };
          contractUri = found.uri;
          contractCid = found.cid;
        }
      } catch {
        // A discovery failure is not fatal; the seat can still paste by hand.
      }
    }
  });

  function describe(e) {
    return e instanceof HandoffError ? e.message : e?.message || String(e);
  }

  function go(build) {
    error = "";
    if (!controller) {
      error = "The atbloons wallet is still connecting. Try again shortly.";
      return;
    }
    try {
      const url = build();
      window.location.assign(url);
    } catch (e) {
      error = describe(e);
      refresh();
    }
  }

  const propose = () =>
    go(() => controller.startProposal({ table: tableRef, soulsPerChip: soulsPerChip.trim() }));
  const activate = () => go(() => controller.startActivation());
  // Settle from the auto-detected terminal action when present, else from the
  // host's manual entry (a fallback until the engine always publishes a
  // settlement-valid terminal action).
  const settle = () =>
    go(() =>
      controller.startSettlement({
        terminalAction: autoTerminal || { uri: terminalUri.trim(), cid: terminalCid.trim() },
      }),
    );
  const withdraw = () => go(() => controller.startWithdrawal());
  const fund = () =>
    go(() =>
      controller.startFunding({
        contract: autoContract || { uri: contractUri.trim(), cid: contractCid.trim() },
      }),
    );

  const sharedContract = $derived(snapshot?.contract || null);
</script>

{#if walletConfigured && tableRef?.uri}
  <section class="atbloons" data-testid="atbloons-handoff">
    <h3>atbloons paid hand</h3>
    <p class="hint">
      Play in chips; escrow and settle in atbloons. The wallet re-verifies everything and holds
      every secret. Testnet only — not real money.
    </p>

    {#if receiptNote}
      <p class="note" data-testid="atbloons-receipt">Last wallet result: {receiptNote}</p>
    {/if}

    {#if !ready}
      <p class="note" data-testid="atbloons-connecting">Connecting to the atbloons wallet…</p>
    {:else if !snapshot || snapshot.grant === "idle"}
      {#if isHost}
        <div class="row">
          <label for="atb-spc">Souls per chip</label>
          <input
            id="atb-spc"
            bind:value={soulsPerChip}
            inputmode="numeric"
            data-testid="atbloons-souls"
          />
          <button class="btn" onclick={propose} data-testid="atbloons-propose"
            >Propose &amp; fund host seat</button
          >
        </div>
      {:else if autoContract}
        <p class="mono" data-testid="atbloons-contract-auto">
          Contract found for this table:<br />{autoContract.uri}
        </p>
        <button class="btn" onclick={fund} data-testid="atbloons-fund">Fund my seat</button>
      {:else}
        <p class="hint">Paste the contract the host shared for this table, then fund your seat.</p>
        <div class="row">
          <label for="atb-cu">Contract AT URI</label>
          <input
            id="atb-cu"
            bind:value={contractUri}
            placeholder="at://…/tech.lenooby09.atbloons.contract/…"
          />
          <label for="atb-cc">Contract CID</label>
          <input id="atb-cc" bind:value={contractCid} placeholder="bafy…" />
          <button class="btn" onclick={fund} data-testid="atbloons-fund-manual">Fund my seat</button
          >
        </div>
      {/if}
    {:else if snapshot.isClosed}
      <p class="note" data-testid="atbloons-closed">Hand grant closed: {snapshot.grant}.</p>
    {:else}
      {#if sharedContract}
        <p class="mono" data-testid="atbloons-contract">
          Contract: {sharedContract.uri}<br />CID: {sharedContract.cid}
        </p>
      {/if}

      {#if snapshot.canWithdraw}
        <button class="btn" onclick={withdraw} data-testid="atbloons-withdraw"
          >Withdraw before activation</button
        >
      {/if}

      {#if snapshot.canActivate}
        <button class="btn" onclick={activate} data-testid="atbloons-activate"
          >Activate (all funded)</button
        >
      {/if}

      {#if snapshot.canSettle}
        {#if autoTerminal}
          <p class="note" data-testid="atbloons-terminal-auto">
            Hand complete. The wallet re-verifies the published action chain and settles the chip
            stacks back to atbloons.
          </p>
          <button class="btn" onclick={settle} data-testid="atbloons-settle">Settle payouts</button>
        {:else}
          <p class="hint">
            The hand has no published terminal action yet. Enter it by hand to settle.
          </p>
          <div class="row">
            <label for="atb-tu">Terminal action AT URI</label>
            <input
              id="atb-tu"
              bind:value={terminalUri}
              placeholder="at://…/re.cardco.poker.action/…"
            />
            <label for="atb-tc">Terminal action CID</label>
            <input id="atb-tc" bind:value={terminalCid} placeholder="bafy…" />
            <button class="btn" onclick={settle} data-testid="atbloons-settle-manual"
              >Settle payouts</button
            >
          </div>
        {/if}
      {/if}
    {/if}

    {#if error}
      <p class="error" data-testid="atbloons-error">{error}</p>
    {/if}
  </section>
{/if}

<style>
  .atbloons {
    border: 3px solid #1a1a1a;
    box-shadow: 6px 6px 0 #1a1a1a;
    padding: 1rem;
    background: #fffef5;
    margin: 1rem 0;
  }
  h3 {
    font-size: 0.6rem;
    letter-spacing: 1px;
    margin-bottom: 0.5rem;
  }
  .hint {
    font-size: 0.4rem;
    opacity: 0.65;
    margin-bottom: 0.6rem;
  }
  .note {
    font-size: 0.42rem;
    margin-bottom: 0.5rem;
  }
  .mono {
    font-family: monospace;
    font-size: 0.4rem;
    word-break: break-all;
    margin-bottom: 0.6rem;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }
  label {
    font-size: 0.4rem;
  }
  input {
    flex: 1;
    min-width: 8rem;
    padding: 0.4rem;
    border: 2px solid #1a1a1a;
    font-family: monospace;
    font-size: 0.42rem;
  }
  .btn {
    padding: 0.5rem 0.9rem;
    border: 2px solid #1a1a1a;
    background: #c0392b;
    color: #fff;
    font-size: 0.42rem;
    letter-spacing: 1px;
    cursor: pointer;
    box-shadow: 3px 3px 0 #1a1a1a;
  }
  .btn:hover {
    transform: translate(2px, 2px);
    box-shadow: 1px 1px 0 #1a1a1a;
  }
  .error {
    color: #c0392b;
    font-size: 0.42rem;
    border: 2px dashed #c0392b;
    padding: 0.4rem;
    margin-top: 0.5rem;
  }
</style>
