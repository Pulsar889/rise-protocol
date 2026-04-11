/**
 * Unstake claimer — scans all WithdrawalTicket accounts and calls claim_unstake
 * on any that have passed their epoch delay.
 *
 * Permissionless: the bot signs as `caller`; SOL is always sent to `owner`
 * (the original unstaker) by the on-chain instruction.
 */
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { RiseClient, PDAS, PROGRAM_IDS, withRetry } from "../client";
import { makeLogger } from "../logger";

const log = makeLogger("unstake-claimer");

// WithdrawalTicket account discriminator: sha256("account:WithdrawalTicket")[..8]
const WITHDRAWAL_TICKET_DISC = Buffer.from([92, 140, 181, 69, 244, 220, 233, 156]);

// WithdrawalTicket layout (65 bytes total):
//   [8  discriminator]
//   [32 owner]
//   [8  sol_amount]
//   [8  claimable_epoch]
//   [8  nonce  (u64 LE)]
//   [1  bump]
const OWNER_OFFSET           = 8;
const CLAIMABLE_EPOCH_OFFSET = 48;
const NONCE_OFFSET           = 56;

function deriveWithdrawalTicketPda(owner: PublicKey, nonce: bigint): PublicKey {
  const nonceBuf = Buffer.alloc(8);
  nonceBuf.writeBigUInt64LE(nonce);
  return PublicKey.findProgramAddressSync(
    [Buffer.from("withdrawal_ticket"), owner.toBuffer(), nonceBuf],
    PROGRAM_IDS.staking
  )[0];
}

export async function runUnstakeClaimer(client: RiseClient): Promise<void> {
  log.info("unstake claimer: scanning withdrawal tickets");

  const { epoch: currentEpoch } = await client.connection.getEpochInfo("confirmed");

  // Fetch all WithdrawalTicket accounts across all users
  let ticketAccounts: Array<{ pubkey: PublicKey; account: { data: Buffer } }>;
  try {
    ticketAccounts = await client.connection.getProgramAccounts(
      PROGRAM_IDS.staking,
      {
        commitment: "confirmed",
        filters: [
          {
            memcmp: {
              offset: 0,
              bytes: WITHDRAWAL_TICKET_DISC.toString("base64"),
              encoding: "base64" as const,
            },
          },
        ],
      }
    ) as unknown as Array<{ pubkey: PublicKey; account: { data: Buffer } }>;
  } catch (err: unknown) {
    log.error("unstake claimer: getProgramAccounts failed", {
      error: err instanceof Error ? err.message : String(err),
    });
    return;
  }

  log.info("unstake claimer: found tickets", { count: ticketAccounts.length, currentEpoch });

  let claimed = 0, pending = 0, failed = 0;

  for (const { pubkey: ticketPubkey, account } of ticketAccounts) {
    const data = account.data as Buffer;

    // Read claimable_epoch (u64 LE, 8 bytes)
    const claimableEpoch = Number(data.readBigUInt64LE(CLAIMABLE_EPOCH_OFFSET));

    if (currentEpoch < claimableEpoch) {
      log.debug("unstake claimer: ticket not yet claimable", {
        ticket: ticketPubkey.toBase58(),
        claimableEpoch,
        currentEpoch,
        epochsRemaining: claimableEpoch - currentEpoch,
      });
      pending++;
      continue;
    }

    // Read owner and nonce
    const owner = new PublicKey(data.slice(OWNER_OFFSET, OWNER_OFFSET + 32));
    const nonce = data.readBigUInt64LE(NONCE_OFFSET);

    // Verify the PDA matches (guards against malformed accounts)
    const expectedPda = deriveWithdrawalTicketPda(owner, nonce);
    if (!expectedPda.equals(ticketPubkey)) {
      log.warn("unstake claimer: ticket PDA mismatch, skipping", {
        ticket:   ticketPubkey.toBase58(),
        expected: expectedPda.toBase58(),
      });
      continue;
    }

    log.info("unstake claimer: claiming ticket", {
      ticket:         ticketPubkey.toBase58(),
      owner:          owner.toBase58(),
      claimableEpoch,
      currentEpoch,
    });

    try {
      await withRetry(async () => {
        await client.staking.methods
          .claimUnstake()
          .accounts({
            caller:        client.wallet.publicKey,
            owner,
            pool:          PDAS.globalPool,
            ticket:        ticketPubkey,
            poolVault:     PDAS.poolVault,
            systemProgram: SystemProgram.programId,
          })
          .rpc({ commitment: "confirmed" });
      }, `claim_unstake(${ticketPubkey.toBase58().slice(0, 8)})`);

      log.info("unstake claimer: ticket claimed", {
        ticket: ticketPubkey.toBase58(),
        owner:  owner.toBase58(),
      });
      claimed++;
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      log.error("unstake claimer: claim failed", {
        ticket: ticketPubkey.toBase58(),
        owner:  owner.toBase58(),
        error:  msg,
      });
      failed++;
    }
  }

  log.info("unstake claimer: scan complete", { claimed, pending, failed });
}
