import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { RiseRewards } from "../target/types/rise_rewards";
import {
  PublicKey,
  SystemProgram,
  LAMPORTS_PER_SOL,
  Keypair,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { assert } from "chai";

describe("rise-rewards", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.RiseRewards as Program<RiseRewards>;
  const authority = provider.wallet as anchor.Wallet;

  let riseMint: PublicKey;
  let rewardsConfig: PublicKey;
  let rewardsVault: PublicKey;
  let gauge: PublicKey;
  let fakePool: PublicKey;

  before(async () => {
    [rewardsConfig] = PublicKey.findProgramAddressSync(
      [Buffer.from("rewards_config")],
      program.programId
    );

    [rewardsVault] = PublicKey.findProgramAddressSync(
      [Buffer.from("rewards_vault")],
      program.programId
    );

    // Reuse existing RISE mint if rewards are already initialized, otherwise create one.
    const configInfo = await provider.connection.getAccountInfo(rewardsConfig);
    if (configInfo !== null) {
      const config = await program.account.rewardsConfig.fetch(rewardsConfig);
      riseMint = config.riseMint;
      console.log("Reusing existing RISE mint:", riseMint.toBase58());
    } else {
      riseMint = await createMint(
        provider.connection,
        authority.payer,
        authority.publicKey,
        null,
        9
      );
    }

    // Always generate a fresh fake pool — a new gauge is created each run.
    fakePool = Keypair.generate().publicKey;

    [gauge] = PublicKey.findProgramAddressSync(
      [Buffer.from("gauge"), fakePool.toBuffer()],
      program.programId
    );

    console.log("RISE mint:", riseMint.toBase58());
    console.log("Rewards config PDA:", rewardsConfig.toBase58());
    console.log("Rewards vault PDA:", rewardsVault.toBase58());
    console.log("Gauge PDA:", gauge.toBase58());
  });

  it("Initializes the rewards program", async () => {
    const epochEmissions = 100_000 * LAMPORTS_PER_SOL;

    const configInfo = await provider.connection.getAccountInfo(rewardsConfig);
    if (configInfo !== null) {
      console.log("Rewards already initialized — skipping");
      const config = await program.account.rewardsConfig.fetch(rewardsConfig);
      assert.equal(config.riseMint.toBase58(), riseMint.toBase58());
      return;
    }

    await program.methods
      .initializeRewards(new anchor.BN(epochEmissions))
      .accounts({
        authority: authority.publicKey,
        config: rewardsConfig,
        riseMint: riseMint,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const config = await program.account.rewardsConfig.fetch(rewardsConfig);

    assert.equal(config.riseMint.toBase58(), riseMint.toBase58());
    assert.equal(config.epochEmissions.toString(), epochEmissions.toString());
    assert.equal(config.currentEpoch.toString(), "0");
    assert.equal(config.gaugeCount.toString(), "0");

    console.log("Rewards program initialized");
    console.log("Epoch emissions:", config.epochEmissions.toString());
    console.log("Slots per epoch:", config.slotsPerEpoch.toString());
  });

  it("Creates a gauge", async () => {
    const gaugeInfo = await provider.connection.getAccountInfo(gauge);
    if (gaugeInfo !== null) {
      console.log("Gauge already exists — skipping");
      return;
    }

    const configBefore = await program.account.rewardsConfig.fetch(rewardsConfig);
    const expectedIndex = configBefore.gaugeCount.toNumber();

    await program.methods
      .createGauge(fakePool)
      .accounts({
        authority: authority.publicKey,
        config: rewardsConfig,
        gauge: gauge,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const gaugeAccount = await program.account.gauge.fetch(gauge);

    assert.equal(gaugeAccount.pool.toBase58(), fakePool.toBase58());
    assert.equal(gaugeAccount.index.toString(), expectedIndex.toString());
    assert.equal(gaugeAccount.active, true);
    assert.equal(gaugeAccount.totalLpDeposited.toString(), "0");

    const config = await program.account.rewardsConfig.fetch(rewardsConfig);
    assert.equal(config.gaugeCount.toNumber(), expectedIndex + 1);

    console.log(`Gauge #${expectedIndex} created for pool:`, fakePool.toBase58());
  });

  it("Updates epoch emissions", async () => {
    const before = await program.account.rewardsConfig.fetch(rewardsConfig);
    const originalEmissions = before.epochEmissions;

    const newEmissions = 200_000 * LAMPORTS_PER_SOL;

    await program.methods
      .setEpochEmissions(new anchor.BN(newEmissions))
      .accounts({
        authority: authority.publicKey,
        config: rewardsConfig,
      })
      .rpc();

    const config = await program.account.rewardsConfig.fetch(rewardsConfig);
    assert.equal(config.epochEmissions.toString(), newEmissions.toString());

    console.log("Epoch emissions updated to:", config.epochEmissions.toString());

    // Reset back to original
    await program.methods
      .setEpochEmissions(originalEmissions)
      .accounts({
        authority: authority.publicKey,
        config: rewardsConfig,
      })
      .rpc();

    const configReset = await program.account.rewardsConfig.fetch(rewardsConfig);
    assert.equal(configReset.epochEmissions.toString(), originalEmissions.toString());
    console.log("Epoch emissions reset to", originalEmissions.toString());
  });

  it("Rewards program has all expected instructions", async () => {
    const names = program.idl.instructions.map((ix: any) => ix.name);

    assert.include(names, "initializeRewards");
    assert.include(names, "createGauge");
    assert.include(names, "checkpointGauge");
    assert.include(names, "depositLp");
    assert.include(names, "withdrawLp");
    assert.include(names, "claimRewards");
    assert.include(names, "setEpochEmissions");

    console.log("All rewards instructions present:", names);
  });

  it("Rewards program has correct account types", async () => {
    const accounts = program.idl.accounts.map((a: any) => a.name);

    assert.include(accounts, "rewardsConfig");
    assert.include(accounts, "gauge");
    assert.include(accounts, "userStake");

    console.log("All rewards account types present:", accounts);
  });
});
