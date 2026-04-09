/**
 * Rise Protocol Crank Bot
 *
 * Runs four independent loops:
 *   - Epoch cranks       (update_exchange_rate, collect_fees, collect_cdp_fees, checkpoint_gauges)
 *   - Reward cranks      (checkpoint_borrow_rewards, checkpoint_stake_rewards)
 *   - Liquidation monitor (accrue_interest + liquidate for unhealthy positions)
 *   - Governance monitor  (execute_proposal for passed + timelocked proposals)
 *
 * Configuration via environment variables — see .env.example.
 */
export {};
//# sourceMappingURL=index.d.ts.map