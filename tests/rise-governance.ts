import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { RiseGovernance } from "../target/types/rise_governance";
import { RiseStaking } from "../target/types/rise_staking";
import {
  PublicKey,
  SystemProgram,
  LAMPORTS_PER_SOL,
  Keypair,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import { SYSVAR_RENT_PUBKEY } from "@solana/web3.js";
import { assert } from "chai";

describe("rise-governance", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const govProgram = anchor.workspace.RiseGovernance as Program<RiseGovernance>;
  const stakingProgram = anchor.workspace.RiseStaking as Program<RiseStaking>;
  const authority = provider.wallet as anchor.Wallet;

  let riseMint: PublicKey;
  let govConfig: PublicKey;
  let riseVault: PublicKey;
  let veLock: PublicKey;
  let gaugeVote: PublicKey;
  let userRiseAccount: PublicKey;
  let treasury: PublicKey;

  // Set during createProposal, used by castVote / closeProposal tests
  let testProposal: PublicKey;

  const NONCE = 0;
  const LOCK_SLOTS = 604_800; // 1 week

  before(async () => {
    [govConfig] = PublicKey.findProgramAddressSync(
      [Buffer.from("governance_config")],
      govProgram.programId
    );
    [riseVault] = PublicKey.findProgramAddressSync(
      [Buffer.from("rise_vault")],
      govProgram.programId
    );
    [veLock] = PublicKey.findProgramAddressSync(
      [Buffer.from("ve_lock"), authority.publicKey.toBuffer(), Buffer.from([NONCE])],
      govProgram.programId
    );
    [gaugeVote] = PublicKey.findProgramAddressSync(
      [Buffer.from("gauge_vote"), authority.publicKey.toBuffer()],
      govProgram.programId
    );
    [treasury] = PublicKey.findProgramAddressSync(
      [Buffer.from("protocol_treasury")],
      stakingProgram.programId
    );

    // If govConfig already exists, reuse its riseMint so account constraints pass.
    const configInfo = await provider.connection.getAccountInfo(govConfig);
    if (configInfo !== null) {
      const config = await govProgram.account.governanceConfig.fetch(govConfig);
      riseMint = config.riseMint;
      const accounts = await provider.connection.getTokenAccountsByOwner(
        authority.publicKey, { mint: riseMint }
      );
      if (accounts.value.length > 0) {
        userRiseAccount = accounts.value[0].pubkey;
      } else {
        userRiseAccount = await createAccount(
          provider.connection, authority.payer, riseMint, authority.publicKey
        );
        await mintTo(
          provider.connection, authority.payer, riseMint,
          userRiseAccount, authority.publicKey, 1_000_000 * LAMPORTS_PER_SOL
        );
      }
    } else {
      riseMint = await createMint(
        provider.connection, authority.payer, authority.publicKey, null, 9
      );
      userRiseAccount = await createAccount(
        provider.connection, authority.payer, riseMint, authority.publicKey
      );
      await mintTo(
        provider.connection, authority.payer, riseMint,
        userRiseAccount, authority.publicKey, 1_000_000 * LAMPORTS_PER_SOL
      );
    }

    console.log("RISE mint:", riseMint.toBase58());
    console.log("Governance config PDA:", govConfig.toBase58());
    console.log("RISE vault PDA:", riseVault.toBase58());
    console.log("VeLock PDA:", veLock.toBase58());
  });

  // ── Initialization ──────────────────────────────────────────────────────────

  it("Initializes governance", async () => {
    const configInfo = await provider.connection.getAccountInfo(govConfig);
    if (configInfo !== null) {
      console.log("Governance already initialized — skipping");
      return;
    }

    // Set proposal threshold to 1 so tests pass without large RISE holdings
    await govProgram.methods
      .initializeGovernance(new anchor.BN(1), 1000)
      .accounts({
        authority: authority.publicKey,
        config: govConfig,
        riseMint: riseMint,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const config = await govProgram.account.governanceConfig.fetch(govConfig);
    assert.equal(config.riseMint.toBase58(), riseMint.toBase58());
    assert.equal(config.quorumBps, 1000);
    console.log("Governance initialized, quorum:", config.quorumBps, "bps");
  });

  it("Initializes RISE vault", async () => {
    const vaultInfo = await provider.connection.getAccountInfo(riseVault);
    if (vaultInfo !== null) {
      console.log("RISE vault already initialized — skipping");
      return;
    }

    await govProgram.methods
      .initializeRiseVault()
      .accounts({
        authority: authority.publicKey,
        config: govConfig,
        riseVault: riseVault,
        riseMint: riseMint,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const vault = await getAccount(provider.connection, riseVault);
    assert.equal(vault.mint.toBase58(), riseMint.toBase58());
    console.log("RISE vault initialized:", riseVault.toBase58());
  });

  // ── Locking ─────────────────────────────────────────────────────────────────

  it("Locks RISE and receives veRISE", async () => {
    const lockAmount = 100_000 * LAMPORTS_PER_SOL;

    const lockInfo = await provider.connection.getAccountInfo(veLock);
    if (lockInfo !== null) {
      const lock = await govProgram.account.veLock.fetch(veLock);
      assert.isTrue(lock.riseLocked.toNumber() > 0);
      console.log("VeLock already exists — skipping. riseLocked:", lock.riseLocked.toString());
      return;
    }

    const beforeBalance = await getAccount(provider.connection, userRiseAccount);

    const nftMintKeypair = Keypair.generate();
    const userNftAta = getAssociatedTokenAddressSync(
      nftMintKeypair.publicKey,
      authority.publicKey
    );

    const TOKEN_METADATA_PROGRAM_ID = new PublicKey("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");
    const [nftMetadata] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("metadata"),
        TOKEN_METADATA_PROGRAM_ID.toBuffer(),
        nftMintKeypair.publicKey.toBuffer(),
      ],
      TOKEN_METADATA_PROGRAM_ID
    );

    await govProgram.methods
      .lockRise(new anchor.BN(lockAmount), new anchor.BN(LOCK_SLOTS), NONCE)
      .accounts({
        user: authority.publicKey,
        config: govConfig,
        lock: veLock,
        userRiseAccount: userRiseAccount,
        riseVault: riseVault,
        nftMint: nftMintKeypair.publicKey,
        userNftAta: userNftAta,
        nftMetadata: nftMetadata,
        tokenMetadataProgram: TOKEN_METADATA_PROGRAM_ID,
        treasury: treasury,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .signers([nftMintKeypair])
      .rpc();

    const lock = await govProgram.account.veLock.fetch(veLock);
    const afterBalance = await getAccount(provider.connection, userRiseAccount);
    const config = await govProgram.account.governanceConfig.fetch(govConfig);
    const transferred = BigInt(beforeBalance.amount) - BigInt(afterBalance.amount);

    assert.equal(transferred.toString(), lockAmount.toString());
    assert.equal(lock.riseLocked.toString(), lockAmount.toString());
    assert.isTrue(lock.veriseAmount.toNumber() > 0);
    assert.isTrue(config.totalVerise.toNumber() > 0);

    console.log("Locked:", lock.riseLocked.toString(), "RISE");
    console.log("veRISE issued:", lock.veriseAmount.toString());
    console.log("Lock expires at slot:", lock.lockEndSlot.toString());
  });

  it("Rejects unlocking a non-expired lock", async () => {
    const lockInfo = await provider.connection.getAccountInfo(veLock);
    if (!lockInfo) {
      console.log("VeLock not found — skipping");
      return;
    }

    const lock = await govProgram.account.veLock.fetch(veLock);
    const currentSlot = await provider.connection.getSlot();

    if (lock.lockEndSlot.toNumber() <= currentSlot) {
      console.log("Lock already expired — skipping negative test");
      return;
    }

    try {
      await govProgram.methods
        .unlockRise()
        .accounts({
          user: authority.publicKey,
          config: govConfig,
          lock: veLock,
          userRiseAccount: userRiseAccount,
          riseVault: riseVault,
          nftMint: lock.nftMint,
          userNftAta: getAssociatedTokenAddressSync(lock.nftMint, authority.publicKey),
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();
      assert.fail("Should have thrown LockNotExpired");
    } catch (err) {
      assert.include(err.toString(), "LockNotExpired");
      console.log("Correctly rejected unlock of non-expired lock");
    }
  });

  it("Extends a lock", async () => {
    const lockBefore = await govProgram.account.veLock.fetch(veLock);
    const oldEndSlot = lockBefore.lockEndSlot.toNumber();

    await govProgram.methods
      .extendLock(new anchor.BN(LOCK_SLOTS))
      .accounts({
        user: authority.publicKey,
        config: govConfig,
        lock: veLock,
      })
      .rpc();

    const lockAfter = await govProgram.account.veLock.fetch(veLock);
    assert.isTrue(lockAfter.lockEndSlot.toNumber() > oldEndSlot);
    console.log("Lock extended to slot:", lockAfter.lockEndSlot.toString());
  });

  // ── Gauge voting ────────────────────────────────────────────────────────────

  it("Records gauge votes", async () => {
    const fakePool1 = Keypair.generate().publicKey;
    const fakePool2 = Keypair.generate().publicKey;

    await govProgram.methods
      .voteGauge([
        { pool: fakePool1, weightBps: 6000 },
        { pool: fakePool2, weightBps: 4000 },
      ])
      .accounts({
        user: authority.publicKey,
        config: govConfig,
        lock: veLock,
        gaugeVote: gaugeVote,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const vote = await govProgram.account.gaugeVote.fetch(gaugeVote);
    assert.equal(vote.gauges[0].weightBps, 6000);
    assert.equal(vote.gauges[1].weightBps, 4000);
    console.log("Gauge votes: Pool 1 -> 6000 bps, Pool 2 -> 4000 bps");
  });

  // ── Config updates ──────────────────────────────────────────────────────────

  it("Updates governance config", async () => {
    const configBefore = await govProgram.account.governanceConfig.fetch(govConfig);
    const originalThreshold = configBefore.proposalThreshold;

    // Change to MIN_PROPOSAL_THRESHOLD (100_000 raw) and back
    const testThreshold = new anchor.BN(100_000);

    await govProgram.methods
      .updateGovernanceConfig(testThreshold, null, null, null)
      .accounts({
        authority: authority.publicKey,
        config: govConfig,
      })
      .rpc({ commitment: "confirmed" });

    const configAfter = await govProgram.account.governanceConfig.fetch(govConfig, "confirmed");
    assert.equal(configAfter.proposalThreshold.toString(), testThreshold.toString());
    console.log("proposal_threshold updated to:", testThreshold.toString());

    // Reset to original
    await govProgram.methods
      .updateGovernanceConfig(originalThreshold, null, null, null)
      .accounts({
        authority: authority.publicKey,
        config: govConfig,
      })
      .rpc({ commitment: "confirmed" });

    const configReset = await govProgram.account.governanceConfig.fetch(govConfig, "confirmed");
    assert.equal(configReset.proposalThreshold.toString(), originalThreshold.toString());
    console.log("proposal_threshold reset to:", originalThreshold.toString());
  });

  // ── Proposals ───────────────────────────────────────────────────────────────

  it("Creates a governance proposal", async () => {
    const govConfigData = await govProgram.account.governanceConfig.fetch(govConfig);

    if (govConfigData.activeProposalCount.toNumber() >= 10) {
      console.log("Active proposal cap reached (10) — skipping");
      return;
    }

    const proposalIndex = govConfigData.proposalCount.toNumber();
    [testProposal] = PublicKey.findProgramAddressSync(
      [Buffer.from("proposal"), govConfigData.proposalCount.toArrayLike(Buffer, "le", 8)],
      govProgram.programId
    );

    const description = new Array(128).fill(0);
    const descText = "Update USDC max LTV to 80%";
    for (let i = 0; i < descText.length; i++) {
      description[i] = descText.charCodeAt(i);
    }

    // create_proposal now takes all locks via remainingAccounts — no single lock account
    await govProgram.methods
      .createProposal(description, Keypair.generate().publicKey)
      .accounts({
        proposer: authority.publicKey,
        config: govConfig,
        proposal: testProposal,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts([
        { pubkey: veLock, isSigner: false, isWritable: false },
      ])
      .rpc();

    const proposalAccount = await govProgram.account.proposal.fetch(testProposal);
    assert.equal(proposalAccount.index.toString(), proposalIndex.toString());
    assert.equal(proposalAccount.executed, false);
    console.log(`Proposal #${proposalIndex} created at ${testProposal.toBase58()}`);
    console.log("Voting ends at slot:", proposalAccount.votingEndSlot.toString());
  });

  it("Casts a vote on proposal", async () => {
    if (!testProposal) {
      console.log("No testProposal set (createProposal skipped) — skipping");
      return;
    }

    // voteRecord seeds: [b"vote_record", lock.key(), proposal.key()]
    const [voteRecord] = PublicKey.findProgramAddressSync(
      [Buffer.from("vote_record"), veLock.toBuffer(), testProposal.toBuffer()],
      govProgram.programId
    );

    const voteRecordInfo = await provider.connection.getAccountInfo(voteRecord);
    if (voteRecordInfo !== null) {
      console.log("Vote already cast — skipping");
      const proposalAccount = await govProgram.account.proposal.fetch(testProposal);
      assert.isTrue(proposalAccount.votesFor.toNumber() > 0);
      return;
    }

    await govProgram.methods
      .castVote(true)
      .accounts({
        voter: authority.publicKey,
        config: govConfig,
        lock: veLock,
        proposal: testProposal,
        voteRecord: voteRecord,
        systemProgram: SystemProgram.programId,
      })
      .rpc({ commitment: "confirmed" });

    const proposalAccount = await govProgram.account.proposal.fetch(testProposal, "confirmed");
    assert.isTrue(proposalAccount.votesFor.toNumber() > 0);
    console.log("Vote cast FOR, votes for:", proposalAccount.votesFor.toString());
  });

  it("Rejects casting a duplicate vote", async () => {
    if (!testProposal) {
      console.log("No testProposal set — skipping");
      return;
    }

    const [voteRecord] = PublicKey.findProgramAddressSync(
      [Buffer.from("vote_record"), veLock.toBuffer(), testProposal.toBuffer()],
      govProgram.programId
    );

    try {
      await govProgram.methods
        .castVote(false)
        .accounts({
          voter: authority.publicKey,
          config: govConfig,
          lock: veLock,
          proposal: testProposal,
          voteRecord: voteRecord,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      assert.fail("Should have thrown AlreadyVoted");
    } catch (err) {
      // "already in use" from Anchor when the account already exists, or AlreadyVoted
      const msg = err.toString();
      assert.isTrue(
        msg.includes("AlreadyVoted") || msg.includes("already in use"),
        `Unexpected error: ${msg}`
      );
      console.log("Correctly rejected duplicate vote");
    }
  });

  it("Rejects closing an active proposal", async () => {
    if (!testProposal) {
      console.log("No testProposal set — skipping");
      return;
    }

    try {
      await govProgram.methods
        .closeProposal()
        .accounts({
          authority: authority.publicKey,
          config: govConfig,
          proposal: testProposal,
        })
        .rpc();
      assert.fail("Should have thrown VotingNotEnded");
    } catch (err) {
      assert.include(err.toString(), "VotingNotEnded");
      console.log("Correctly rejected closing an active proposal");
    }
  });

  // ── IDL completeness ────────────────────────────────────────────────────────

  it("Governance program has all expected instructions", async () => {
    const names = govProgram.idl.instructions.map((ix: any) => ix.name);
    assert.include(names, "initializeGovernance");
    assert.include(names, "initializeRiseVault");
    assert.include(names, "lockRise");
    assert.include(names, "unlockRise");
    assert.include(names, "extendLock");
    assert.include(names, "voteGauge");
    assert.include(names, "createProposal");
    assert.include(names, "castVote");
    assert.include(names, "closeProposal");
    assert.include(names, "executeProposal");
    assert.include(names, "claimRevenueShare");
    assert.include(names, "updateGovernanceConfig");
    console.log("All governance instructions present");
  });
});
