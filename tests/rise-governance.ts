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

  it("Initializes governance", async () => {
    const configInfo = await provider.connection.getAccountInfo(govConfig);
    if (configInfo !== null) {
      console.log("Governance already initialized — skipping");
      return;
    }

    // Set proposal threshold to 1 lamport so tests pass easily
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

  it("Locks RISE and receives veRISE", async () => {
    const lockAmount = 100_000 * LAMPORTS_PER_SOL;

    // Skip if lock already exists (persistent validator re-run)
    const lockInfo = await provider.connection.getAccountInfo(veLock);
    if (lockInfo !== null) {
      const lock = await govProgram.account.veLock.fetch(veLock);
      assert.isTrue(lock.riseLocked.toNumber() > 0);
      console.log("VeLock already exists — skipping. riseLocked:", lock.riseLocked.toString());
      return;
    }

    const beforeBalance = await getAccount(provider.connection, userRiseAccount);

    // Fresh NFT mint keypair for this lock position
    const nftMintKeypair = Keypair.generate();

    // ATA for the NFT (user receives 1 token)
    const userNftAta = getAssociatedTokenAddressSync(
      nftMintKeypair.publicKey,
      authority.publicKey
    );

    // Metaplex metadata PDA: seeds = ["metadata", TOKEN_METADATA_PROGRAM_ID, nft_mint]
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

  it("Creates a governance proposal", async () => {
    // Proposal #0 is the canonical proposal for this test suite.
    // Check if it already exists before attempting creation.
    const [proposal0] = PublicKey.findProgramAddressSync(
      [Buffer.from("proposal"), new anchor.BN(0).toArrayLike(Buffer, "le", 8)],
      govProgram.programId
    );
    const proposal0Info = await provider.connection.getAccountInfo(proposal0);
    if (proposal0Info !== null) {
      console.log("Proposal #0 already exists — skipping");
      const proposalAccount = await govProgram.account.proposal.fetch(proposal0);
      assert.equal(proposalAccount.index.toString(), "0");
      assert.equal(proposalAccount.executed, false);
      return;
    }

    // Derive the proposal PDA from the current on-chain proposal_count so the seeds
    // match what the program will use at init time (seeds = [b"proposal", count.to_le_bytes()]).
    const govConfigData = await govProgram.account.governanceConfig.fetch(govConfig);
    const [proposal] = PublicKey.findProgramAddressSync(
      [Buffer.from("proposal"), govConfigData.proposalCount.toArrayLike(Buffer, "le", 8)],
      govProgram.programId
    );

    const description = new Array(128).fill(0);
    const descText = "Update USDC max LTV to 80%";
    for (let i = 0; i < descText.length; i++) {
      description[i] = descText.charCodeAt(i);
    }

    await govProgram.methods
      .createProposal(description, Keypair.generate().publicKey)
      .accounts({
        proposer: authority.publicKey,
        config: govConfig,
        lock: veLock,
        proposal: proposal,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const proposalAccount = await govProgram.account.proposal.fetch(proposal);
    assert.equal(proposalAccount.index.toString(), govConfigData.proposalCount.toString());
    assert.equal(proposalAccount.executed, false);
    console.log(`Proposal #${govConfigData.proposalCount} created`);
    console.log("Voting ends at slot:", proposalAccount.votingEndSlot.toString());
  });

  it("Casts a vote on proposal", async () => {
    // Always vote on proposal #0.
    const [proposal] = PublicKey.findProgramAddressSync(
      [Buffer.from("proposal"), new anchor.BN(0).toArrayLike(Buffer, "le", 8)],
      govProgram.programId
    );

    const [voteRecord] = PublicKey.findProgramAddressSync(
      [Buffer.from("vote_record"), authority.publicKey.toBuffer(), proposal.toBuffer()],
      govProgram.programId
    );

    const voteRecordInfo = await provider.connection.getAccountInfo(voteRecord);
    if (voteRecordInfo !== null) {
      console.log("Vote already cast — skipping");
      const proposalAccount = await govProgram.account.proposal.fetch(proposal);
      assert.isTrue(proposalAccount.votesFor.toNumber() > 0);
      return;
    }

    await govProgram.methods
      .castVote(true)
      .accounts({
        voter: authority.publicKey,
        config: govConfig,
        lock: veLock,
        proposal: proposal,
        voteRecord: voteRecord,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const proposalAccount = await govProgram.account.proposal.fetch(proposal);
    assert.isTrue(proposalAccount.votesFor.toNumber() > 0);
    console.log("Vote cast FOR, votes for:", proposalAccount.votesFor.toString());
  });

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
    assert.include(names, "executeProposal");
    assert.include(names, "claimRevenueShare");
    console.log("All governance instructions present");
  });
});
