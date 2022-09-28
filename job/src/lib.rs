#![no_std]
elrond_wasm::imports!();

pub mod status;
pub mod base;
pub mod constants;

use status::EscrowStatus;
use constants::OraclePair;
use constants::UrlHashPair;

#[elrond_wasm::contract]
pub trait JobContract: base::JobBaseModule {

    #[init]
    fn init(
        &self,
        token: EgldOrEsdtTokenIdentifier,
        canceller: ManagedAddress,
        duration: u64,
        trusted_callers: MultiValueEncoded<ManagedAddress>
    ) {
        self.init_base(token, duration, trusted_callers);
        self.launcher().set(self.blockchain().get_caller());
        self.canceller().set(canceller);
    }

    #[endpoint]
    fn setup(
        &self,
        reputation_oracle: ManagedAddress,
        reputation_oracle_stake: BigUint,
        recording_oracle: ManagedAddress,
        recording_oracle_stake: BigUint,
        url: ManagedBuffer,
        hash: ManagedBuffer,
    ) {
        self.require_trusted();
        self.require_not_expired();
        require!(self.status().get() == EscrowStatus::Launched, "Contract is not launched");

        let total_stake = &reputation_oracle_stake + &recording_oracle_stake;
        require!(total_stake <= 100_u64, "Stake out of bounds");

        self.oracle_pair().set(&OraclePair::new(
            &reputation_oracle,
            reputation_oracle_stake,
            &recording_oracle,
            recording_oracle_stake
        ));

        self.trusted_callers().insert(recording_oracle);
        self.trusted_callers().insert(reputation_oracle);
        self.manifest().set(&UrlHashPair::new(url, hash));
        self.status().set(EscrowStatus::Pending);
        // self.pending_event(url, hash);
    }

    #[endpoint]
    fn cancel(&self) -> () {
        self.require_not_status(&[EscrowStatus::Complete, EscrowStatus::Paid]);
        self.require_not_broke();

        let balance: BigUint = self.get_balance();
        self.send().direct(&self.canceller().get(), &self.token().get(), 0, &balance);

        self.status().set(EscrowStatus::Cancelled)
    }

    #[endpoint]
    fn abort(&self) -> () {
        self.require_not_status(&[EscrowStatus::Complete, EscrowStatus::Paid]);
        let balance: BigUint = self.get_balance();
        if balance != 0 {
            self.cancel()
        } else {
            self.status().set(EscrowStatus::Cancelled)
        }
    }

    #[endpoint]
    fn complete(&self) -> () {
        self.require_trusted();
        self.require_not_expired();
        self.required_status(&[EscrowStatus::Paid]);
        self.status().set(EscrowStatus::Complete);
    }

    #[endpoint(storeResults)]
    fn store_results(&self, url: ManagedBuffer, hash: ManagedBuffer) {
        self.require_trusted();
        self.require_not_expired();
        self.required_status(&[EscrowStatus::Pending, EscrowStatus::Partial]);
        self.intermediate_results().set(UrlHashPair::new(url, hash));
    }

    #[view(getIntermediateResults)]
    fn get_intermediate_results(&self) -> UrlHashPair<Self::Api> {
        require!(!self.intermediate_results().is_empty(), "intermediate results are not set");
        self.intermediate_results().get()
    }

    #[view(getFinalResults)]
    fn get_final_results(&self) -> UrlHashPair<Self::Api> {
        require!(!self.final_results().is_empty(), "final results are not set");
        self.final_results().get()
    }

    fn transfer_fee(
        &self,
        mut from_amount: BigUint,
        mut to_amount: BigUint,
        original_amount: &BigUint,
        percentage: &BigUint,
    ) -> (BigUint, BigUint) {
        let transferred_amount = original_amount * percentage / BigUint::from(100_u64);
        from_amount -= &transferred_amount;
        to_amount += &transferred_amount;
        (from_amount, to_amount)
    }

    #[endpoint(bulkPayOut)]
    fn bulk_pay_out(
        &self,
        payments: MultiValueEncoded<(ManagedAddress, BigUint)>,
        final_results: OptionalValue<UrlHashPair<Self::Api>>,
    ) {
        self.require_trusted();
        self.require_not_expired();
        self.require_not_status(&[EscrowStatus::Paid, EscrowStatus::Launched]);
        self.require_not_broke();

        let token = self.token().get();
        let oracles = self.oracle_pair().get();

        let mut recording_fee = BigUint::zero();
        let mut reputation_fee = BigUint::zero();

        for (to, amount) in payments {
            let mut payout = amount.clone();
            (payout, reputation_fee) = self.transfer_fee(payout, reputation_fee, &amount, &oracles.reputation.stake);
            (payout, recording_fee) = self.transfer_fee(payout, recording_fee, &amount, &oracles.recording.stake);
            self.send().direct(&to, &token, 0, &payout);
        }

        self.send().direct(&oracles.reputation.address, &token, 0, &reputation_fee);
        self.send().direct(&oracles.recording.address, &token, 0, &recording_fee);

        let next_status = if self.get_balance() != 0 {
            EscrowStatus::Partial
        } else {
            EscrowStatus::Paid
        };
        self.status().set(&next_status);

        if let Some(results) = final_results.into_option() {
            self.final_results().set(&results);
        }
    }

    #[storage_mapper("launcher")]
    fn launcher(&self) -> SingleValueMapper<ManagedAddress>;

    #[storage_mapper("canceller")]
    fn canceller(&self) -> SingleValueMapper<ManagedAddress>;

}
