import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { RiseStaking } from "../target/types/rise_staking";
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
  getAccount,
} from "@solana/spl-token";
import { assert } from "chai";

const MIN_DEPLOYER_BALANCE = LAMPORTS_PER_SOL; // 1 SOL

describe("rise-staking", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.RiseStaking as Program<RiseStaking>;
  const authority = provider.wallet as anchor.Wallet;

  let riseSolMint: PublicKey;
  let globalPool: PublicKey;
  let poolVault: PublicKey;
  let treasury: PublicKey;
  let treasuryVault: PublicKey;
  let userRiseSolAccount: PublicKey;
  let teamWallet: Keypair;

  before(async () => {
    const balance = await provider.connection.getBalance(authority.publicKey);
    assert.isTrue(
      balance >= MIN_DEPLOYER_BALANCE,
      `Deployer wallet needs ≥ 1 SOL for devnet tests, current balance: ${balance / LAMPORTS_PER_SOL} SOL`
    );

    [globalPool] = PublicKey.findProgramAddressSync(
      [Buffer.from("global_pool")],
      program.programId
    );
    [poolVault] = PublicKey.findProgramAddressSync(
      [Buffer.from("pool_vault")],
      program.programId
    );
    [treasury] = PublicKey.findProgramAddressSync(
      [Buffer.from("protocol_treasury")],
      program.programId
    );
    [treasuryVault] = PublicKey.findProgramAddressSync(
      [Buffer.from("treasury_vault")],
      program.programId
    );

    // Generate a team wallet keypair — used only as a stored address, needs no SOL
    teamWallet = Keypair.generate();

    const poolInfo = await provider.connection.getAccountInfo(globalPool);
    if (poolInfo !== null) {
      const poolAccount = await program.account.globalPool.fetch(globalPool);
      riseSolMint = poolAccount.riseSolMint;
    } else {
      riseSolMint = await createMint(
        provider.connection,
        authority.payer,
        globalPool,
        null,
        9
      );
    }

    try {
      userRiseSolAccount = await createAccount(
        provider.connection,
        authority.payer,
        riseSolMint,
        authority.publicKey
      );
    } catch {
      const accounts = await provider.connection.getTokenAccountsByOwner(
        authority.publicKey,
        { mint: riseSolMint }
      );
      userRiseSolAccount = accounts.value[0].pubkey;
    }

    console.log("Global pool PDA:", globalPool.toBase58());
    console.log("Treasury PDA:", treasury.toBase58());
    console.log("Treasury vault PDA:", treasuryVault.toBase58());
    console.log("Team wallet:", teamWallet.publicKey.toBase58());
  });

  it("Initializes the staking pool", async () => {
    const poolInfo = await provider.connection.getAccountInfo(globalPool);
    if (poolInfo !== null) {
      console.log("Pool already initialized — skipping");
      const poolAccount = await program.account.globalPool.fetch(globalPool);
      assert.equal(poolAccount.exchangeRate.toString(), "1000000000");
      return;
    }
    await program.methods
      .initializePool(1000, 500)
      .accounts({
        authority: authority.publicKey,
        pool: globalPool,
        riseSolMint: riseSolMint,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
    const poolAccount = await program.account.globalPool.fetch(globalPool);
    assert.equal(poolAccount.exchangeRate.toString(), "1000000000");
    console.log("Pool initialized");
  });

  it("Initializes the protocol treasury", async () => {
    const treasuryInfo = await provider.connection.getAccountInfo(treasury);
    if (treasuryInfo !== null) {
      console.log("Treasury already initialized — skipping");
      return;
    }

    await program.methods
      .initializeTreasury(
        teamWallet.publicKey,
        500,  // 5% team fee
        5000  // 50% veRISE share
      )
      .accounts({
        authority: authority.publicKey,
        treasury: treasury,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const treasuryAccount = await program.account.protocolTreasury.fetch(treasury);

    assert.equal(
      treasuryAccount.teamWallet.toBase58(),
      teamWallet.publicKey.toBase58()
    );
    assert.equal(treasuryAccount.teamFeeBps, 500);
    assert.equal(treasuryAccount.veriseShareBps, 5000);
    assert.equal(treasuryAccount.reserveLamports.toString(), "0");

    console.log("Treasury initialized");
    console.log("Team fee:", treasuryAccount.teamFeeBps, "bps");
    console.log("veRISE share:", treasuryAccount.veriseShareBps, "bps");
  });

  it("Stakes SOL and receives riseSOL", async () => {
    const stakeAmount = LAMPORTS_PER_SOL;
    const before = await getAccount(provider.connection, userRiseSolAccount);

    await program.methods
      .stakeSol(new anchor.BN(stakeAmount))
      .accounts({
        user: authority.publicKey,
        pool: globalPool,
        poolVault: poolVault,
        riseSolMint: riseSolMint,
        userRiseSolAccount: userRiseSolAccount,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        stakeRewardsConfig: null,
        userStakeRewards: null,
      })
      .rpc();

    const after = await getAccount(provider.connection, userRiseSolAccount);
    const minted = BigInt(after.amount) - BigInt(before.amount);
    assert.equal(minted.toString(), stakeAmount.toString());
    console.log("Staked 1 SOL, received riseSOL:", minted.toString());
  });

  it("Updates treasury team fee", async () => {
    const before = await program.account.protocolTreasury.fetch(treasury);
    const originalFee = before.teamFeeBps;

    await program.methods
      .updateTreasuryConfig(
        null,   // team_wallet — no change
        800,    // team_fee_bps — update to 8%
        null    // verise_share_bps — no change
      )
      .accounts({
        authority: authority.publicKey,
        treasury: treasury,
      })
      .rpc();

    const treasuryAccount = await program.account.protocolTreasury.fetch(treasury, "confirmed");
    assert.equal(treasuryAccount.teamFeeBps, 800);
    console.log("Team fee updated to:", treasuryAccount.teamFeeBps, "bps");

    // Reset back to original
    await program.methods
      .updateTreasuryConfig(null, originalFee, null)
      .accounts({
        authority: authority.publicKey,
        treasury: treasury,
      })
      .rpc();

    const treasuryReset = await program.account.protocolTreasury.fetch(treasury, "confirmed");
    assert.equal(treasuryReset.teamFeeBps, originalFee);
    console.log("Team fee reset to", originalFee, "bps");
  });

  it("Updates treasury veRISE share", async () => {
    await program.methods
      .updateTreasuryConfig(
        null,   // team_wallet
        null,   // team_fee_bps
        7000    // verise_share_bps — update to 70%
      )
      .accounts({
        authority: authority.publicKey,
        treasury: treasury,
      })
      .rpc({ commitment: "confirmed" });

    const treasuryAccount = await program.account.protocolTreasury.fetch(treasury, "confirmed");
    assert.equal(treasuryAccount.veriseShareBps, 7000);
    console.log("veRISE share updated to:", treasuryAccount.veriseShareBps, "bps");

    // Reset back to 50%
    await program.methods
      .updateTreasuryConfig(null, null, 5000)
      .accounts({
        authority: authority.publicKey,
        treasury: treasury,
      })
      .rpc({ commitment: "confirmed" });

    const treasuryReset = await program.account.protocolTreasury.fetch(treasury, "confirmed");
    assert.equal(treasuryReset.veriseShareBps, 5000);
    console.log("veRISE share reset to 5000 bps");
  });

  it("Unstakes riseSOL and creates a WithdrawalTicket", async () => {
    const unstakeAmount = LAMPORTS_PER_SOL;

    const UNSTAKE_NONCE = 0;
    const [withdrawalTicket] = PublicKey.findProgramAddressSync(
      [Buffer.from("withdrawal_ticket"), authority.publicKey.toBuffer(), Buffer.from([UNSTAKE_NONCE])],
      program.programId
    );

    // Skip creation if the ticket already exists (persistent validator re-run).
    const ticketInfo = await provider.connection.getAccountInfo(withdrawalTicket);
    if (ticketInfo !== null) {
      const ticket = await program.account.withdrawalTicket.fetch(withdrawalTicket);
      assert.equal(ticket.owner.toBase58(), authority.publicKey.toBase58());
      console.log(`WithdrawalTicket already exists — skipping. ${ticket.solAmount.toNumber() / LAMPORTS_PER_SOL} SOL claimable at epoch ${ticket.claimableEpoch}`);
      return;
    }

    await program.methods
      .unstakeRiseSol(new anchor.BN(unstakeAmount), UNSTAKE_NONCE)
      .accounts({
        user: authority.publicKey,
        pool: globalPool,
        ticket: withdrawalTicket,
        riseSolMint: riseSolMint,
        userRiseSolAccount: userRiseSolAccount,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    // Unstaking now creates a WithdrawalTicket instead of returning SOL immediately.
    // Verify the ticket was created with the correct owner and amount.
    const ticket = await program.account.withdrawalTicket.fetch(withdrawalTicket);
    assert.equal(ticket.owner.toBase58(), authority.publicKey.toBase58());
    assert.equal(ticket.solAmount.toString(), unstakeAmount.toString());
    assert.isTrue(ticket.claimableEpoch.toNumber() > 0);
    console.log(
      `WithdrawalTicket created: ${unstakeAmount / LAMPORTS_PER_SOL} SOL claimable at epoch ${ticket.claimableEpoch}`
    );
  });

  it("Rejects staking zero SOL", async () => {
    try {
      await program.methods
        .stakeSol(new anchor.BN(0))
        .accounts({
          user: authority.publicKey,
          pool: globalPool,
          poolVault: poolVault,
          riseSolMint: riseSolMint,
          userRiseSolAccount: userRiseSolAccount,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          stakeRewardsConfig: null,
          userStakeRewards: null,
        })
        .rpc();
      assert.fail("Should have thrown");
    } catch (err) {
      assert.include(err.toString(), "ZeroAmount");
      console.log("Correctly rejected zero amount");
    }
  });
});
