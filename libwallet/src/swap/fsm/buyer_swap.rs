// Copyright 2020 The MWC Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// Sell swap happy path states

use crate::swap::bitcoin::{BtcNodeClient, BtcSwapApi};
use crate::swap::fsm::state::{Input, State, StateId, StateProcessRespond};
use crate::swap::message::Message;
use crate::swap::swap;
use crate::swap::swap::publish_transaction;
use crate::swap::types::{Action, SwapTransactionsConfirmations};
use crate::swap::{BuyApi, Context, ErrorKind, Swap, SwapApi};
use crate::NodeClient;
use failure::_core::marker::PhantomData;
use grin_core::core::verifier_cache::LruVerifierCache;
use grin_core::core::Weighting;
use grin_keychain::Keychain;
use grin_util::RwLock;
use std::sync::Arc;

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerOfferCreated
pub struct BuyerOfferCreated {}
impl BuyerOfferCreated {
	/// Create new instance
	pub fn new() -> Self {
		Self {}
	}
}
impl State for BuyerOfferCreated {
	fn get_state_id(&self) -> StateId {
		StateId::BuyerOfferCreated
	}
	fn get_name(&self) -> String {
		"Get an Offer".to_string()
	}
	fn is_cancellable(&self) -> bool {
		true
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		_swap: &mut Swap,
		_context: &Context,
		_tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Cancel => Ok(StateProcessRespond::new(StateId::BuyerCancelled)),
			Input::Check => Ok(StateProcessRespond::new(
				StateId::BuyerSendingAcceptOfferMessage,
			)),
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerOfferCreated get {:?}",
				input
			))),
		}
	}

	fn get_prev_swap_state(&self) -> Option<StateId> {
		None
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerSendingAcceptOfferMessage)
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerSendingAcceptOfferMessage
pub struct BuyerSendingAcceptOfferMessage<'a, C, B, K>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
	K: Keychain + 'a,
{
	keychain: Arc<K>,
	swap_api: Arc<BtcSwapApi<'a, C, B>>,
	message: Option<Message>,
	phantom: PhantomData<&'a C>,
}
impl<'a, C, B, K> BuyerSendingAcceptOfferMessage<'a, C, B, K>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
	K: Keychain + 'a,
{
	/// Create new instance
	pub fn new(keychain: Arc<K>, swap_api: Arc<BtcSwapApi<'a, C, B>>) -> Self {
		Self {
			keychain,
			swap_api,
			phantom: PhantomData,
			message: None,
		}
	}
}
impl<'a, C, B, K> State for BuyerSendingAcceptOfferMessage<'a, C, B, K>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
	K: Keychain + 'a,
{
	fn get_state_id(&self) -> StateId {
		StateId::BuyerSendingAcceptOfferMessage
	}
	fn get_name(&self) -> String {
		"Sending Accept Offer Message".to_string()
	}
	fn is_cancellable(&self) -> bool {
		true
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		_context: &Context,
		_tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Cancel => Ok(StateProcessRespond::new(StateId::BuyerCancelled)),
			Input::Check => {
				let time_limit = swap.get_time_message_offers();
				if swap.message1.is_none() {
					if swap::get_cur_time() < time_limit {
						if self.message.is_none() {
							let sec_update = self
								.swap_api
								.build_accept_offer_message_secondary_update(&*self.keychain, swap);
							self.message = Some(BuyApi::accept_offer_message(swap, sec_update)?);
						}
						Ok(
							StateProcessRespond::new(StateId::BuyerSendingAcceptOfferMessage)
								.action(Action::BuyerSendAcceptOfferMessage(
									self.message.clone().unwrap(),
								))
								.time_limit(time_limit),
						)
					} else {
						Ok(StateProcessRespond::new(StateId::BuyerCancelled))
					}
				} else {
					// Probably it is a rerun because of some reset. We should tolerate that
					Ok(StateProcessRespond::new(
						StateId::BuyerWaitingForSellerToLock,
					))
				}
			}
			Input::Execute {
				refund_address: _,
				fee_satoshi_per_byte: _,
			} => {
				debug_assert!(swap.message1.is_none());
				debug_assert!(self.message.is_some()); // Check expected to be called first
				swap.message1 = Some(self.message.clone().unwrap());
				swap.posted_msg1 = Some(swap::get_cur_time());
				Ok(StateProcessRespond::new(
					StateId::BuyerWaitingForSellerToLock,
				))
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerSendingAcceptOfferMessage get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerOfferCreated)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitingForSellerToLock)
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerWaitingForSellerToLock
pub struct BuyerWaitingForSellerToLock {}
impl BuyerWaitingForSellerToLock {
	/// Create new instance
	pub fn new() -> Self {
		Self {}
	}
}

impl State for BuyerWaitingForSellerToLock {
	fn get_state_id(&self) -> StateId {
		StateId::BuyerWaitingForSellerToLock
	}
	fn get_name(&self) -> String {
		"Waiting For Seller to start Locking MWC".to_string()
	}
	fn is_cancellable(&self) -> bool {
		true
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		_context: &Context,
		tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Cancel => Ok(StateProcessRespond::new(StateId::BuyerCancelled)),
			Input::Check => {
				let time_limit = swap.get_time_start_lock();
				// Check the deadline for locking
				if swap::get_cur_time() > time_limit {
					// cancelling
					return Ok(StateProcessRespond::new(StateId::BuyerCancelled));
				}

				if !swap.seller_lock_first {
					// Skipping this step. Seller waiting for us to start locking
					Ok(StateProcessRespond::new(
						StateId::BuyerPostingSecondaryToMultisigAccount,
					))
				} else {
					let conf = tx_conf.mwc_lock_conf.unwrap_or(0);

					if conf < 1 {
						Ok(
							StateProcessRespond::new(StateId::BuyerWaitingForSellerToLock)
								.action(Action::WaitForMwcConfirmations {
									name: "Seller to lock funds".to_string(),
									required: 1,
									actual: conf,
								})
								.time_limit(time_limit),
						)
					} else {
						Ok(StateProcessRespond::new(
							StateId::BuyerPostingSecondaryToMultisigAccount,
						))
					}
				}
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerWaitingForSellerToLock get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerSendingAcceptOfferMessage)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerPostingSecondaryToMultisigAccount)
	}
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerPostingSecondaryToMultisigAccount
pub struct BuyerPostingSecondaryToMultisigAccount<'a, C, B>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
{
	swap_api: Arc<BtcSwapApi<'a, C, B>>,
	phantom: PhantomData<&'a C>,
}
impl<'a, C, B> BuyerPostingSecondaryToMultisigAccount<'a, C, B>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
{
	/// Create new instance
	pub fn new(swap_api: Arc<BtcSwapApi<'a, C, B>>) -> Self {
		Self {
			swap_api,
			phantom: PhantomData,
		}
	}
}

impl<'a, C, B> State for BuyerPostingSecondaryToMultisigAccount<'a, C, B>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
{
	fn get_state_id(&self) -> StateId {
		StateId::BuyerPostingSecondaryToMultisigAccount
	}
	fn get_name(&self) -> String {
		"Waiting for posting BTC to lock account".to_string()
	}
	fn is_cancellable(&self) -> bool {
		true
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		_context: &Context,
		_tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Cancel => Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime)), // We can't just cancel. Funds might be posted, it is a manual process.
			Input::Check => {
				// Check if mwc lock is already done
				let input_script = self.swap_api.script(swap)?;

				let (pending_amount, confirmed_amount, _least_confirmations, _outputs) = self
					.swap_api
					.btc_balance(swap, &input_script, swap.secondary_confirmations)?;

				let chain_amount = pending_amount + confirmed_amount;
				let time_limit = swap.get_time_start_lock();

				// Check the deadline for locking
				if chain_amount != swap.secondary_amount {
					if swap::get_cur_time() > time_limit {
						// cancelling because of timeout
						return Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime));
					}
				}

				if chain_amount < swap.secondary_amount {
					// At this point, user needs to deposit (more) Bitcoin
					return Ok(StateProcessRespond::new(
						StateId::BuyerPostingSecondaryToMultisigAccount,
					)
					.action(Action::DepositSecondary {
						currency: swap.secondary_currency,
						amount: swap.secondary_amount - chain_amount,
						address: format!(
							"{}",
							swap.secondary_data
								.unwrap_btc()?
								.address(&input_script, swap.network)?
						),
					})
					.time_limit(time_limit));
				}

				// Posted more then expected. We are not going forward. Deal is broken, probably it is a mistake. We are cancelling the trade because of that.
				if chain_amount > swap.secondary_amount {
					return Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime));
				}

				debug_assert!(chain_amount == swap.secondary_amount);

				Ok(StateProcessRespond::new(
					StateId::BuyerWaitingForLockConfirmations,
				))
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerPostingSecondaryToMultisigAccount get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitingForSellerToLock)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitingForLockConfirmations)
	}
}

///////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerWaitingForLockConfirmations
pub struct BuyerWaitingForLockConfirmations<K>
where
	K: Keychain,
{
	keychain: Arc<K>,
}
impl<K> BuyerWaitingForLockConfirmations<K>
where
	K: Keychain,
{
	/// Create new instance
	pub fn new(keychain: Arc<K>) -> Self {
		Self { keychain }
	}
}

impl<K> State for BuyerWaitingForLockConfirmations<K>
where
	K: Keychain,
{
	fn get_state_id(&self) -> StateId {
		StateId::BuyerWaitingForLockConfirmations
	}
	fn get_name(&self) -> String {
		"Waiting for Locking funds confirmations".to_string()
	}
	fn is_cancellable(&self) -> bool {
		true
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		context: &Context,
		tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Cancel => Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime)), // Long cancellation path
			Input::Check => {
				let mwc_lock = tx_conf.mwc_lock_conf.unwrap_or(0);
				let secondary_lock = tx_conf.secondary_lock_conf.unwrap_or(0);

				if tx_conf.secondary_lock_amount < swap.secondary_amount {
					// Need to deposit more. Something happens? Likely will be cancelled because of timeout.
					return Ok(StateProcessRespond::new(
						StateId::BuyerPostingSecondaryToMultisigAccount,
					));
				}

				if tx_conf.secondary_lock_amount > swap.secondary_amount {
					// Posted too much, bayer probably will cancel the deal, let's be in sync
					return Ok(StateProcessRespond::new(
						StateId::SellerWaitingForRefundHeight,
					));
				}

				let time_limit = swap.get_time_message_redeem();
				if mwc_lock < swap.mwc_confirmations
					|| secondary_lock < swap.secondary_confirmations
				{
					// Checking for a deadline. Note time_message_redeem is fine, we can borrow time from that operation and still be safe
					if swap::get_cur_time() > time_limit {
						// cancelling because of timeout
						return Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime));
					}
				}

				// Every party waiting first on there own funds. For buyer it is BTC
				if secondary_lock < swap.secondary_confirmations {
					return Ok(
						StateProcessRespond::new(StateId::BuyerWaitingForLockConfirmations)
							.action(Action::WaitForSecondaryConfirmations {
								name: format!("{} Locking Account", swap.secondary_currency),
								currency: swap.secondary_currency,
								required: swap.secondary_confirmations,
								actual: secondary_lock,
							})
							.time_limit(time_limit),
					);
				}

				if mwc_lock < swap.mwc_confirmations {
					return Ok(
						StateProcessRespond::new(StateId::BuyerWaitingForLockConfirmations)
							.action(Action::WaitForMwcConfirmations {
								name: "MWC Lock transaction".to_string(),
								required: swap.mwc_confirmations,
								actual: mwc_lock,
							})
							.time_limit(time_limit),
					);
				}

				// If we got here, funds have been locked on both chains with sufficient confirmations
				// On the first run - let's update the swap data
				if swap.redeem_slate.participant_data.len() <= 1 || swap.adaptor_signature.is_none()
				{
					BuyApi::init_redeem(&*self.keychain, swap, context)?;
				}

				Ok(StateProcessRespond::new(
					StateId::BuyerSendingInitRedeemMessage,
				))
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerWaitingForLockConfirmations get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerPostingSecondaryToMultisigAccount)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerSendingInitRedeemMessage)
	}
}

/////////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerSendingInitRedeemMessage
pub struct BuyerSendingInitRedeemMessage {
	message: Option<Message>,
}
impl BuyerSendingInitRedeemMessage {
	/// Create new instance
	pub fn new() -> Self {
		Self { message: None }
	}
}
impl State for BuyerSendingInitRedeemMessage {
	fn get_state_id(&self) -> StateId {
		StateId::BuyerSendingInitRedeemMessage
	}
	fn get_name(&self) -> String {
		"Sending Init Redeem Message".to_string()
	}
	fn is_cancellable(&self) -> bool {
		true
	}

	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		_context: &Context,
		tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Cancel => Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime)), // Last chance to quit
			Input::Check => {
				// Check first if everything is still locked...
				let mwc_lock = tx_conf.mwc_lock_conf.unwrap_or(0);
				let secondary_lock = tx_conf.secondary_lock_conf.unwrap_or(0);
				if mwc_lock < swap.mwc_confirmations
					|| secondary_lock < swap.secondary_confirmations
				{
					return Ok(StateProcessRespond::new(
						StateId::BuyerWaitingForLockConfirmations,
					));
				}

				let time_limit = swap.get_time_message_redeem();
				if swap.message2.is_none() {
					if swap::get_cur_time() < time_limit {
						if self.message.is_none() {
							self.message = Some(BuyApi::init_redeem_message(swap)?);
						}
						Ok(
							StateProcessRespond::new(StateId::BuyerSendingInitRedeemMessage)
								.action(Action::BuyerSendInitRedeemMessage(
									self.message.clone().unwrap(),
								))
								.time_limit(time_limit),
						)
					} else {
						Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime))
					}
				} else {
					// Probably it is a rerun because of some reset. We should tolerate that
					Ok(StateProcessRespond::new(
						StateId::BuyerWaitingForRespondRedeemMessage,
					))
				}
			}
			Input::Execute {
				refund_address: _,
				fee_satoshi_per_byte: _,
			} => {
				debug_assert!(swap.message2.is_none());
				debug_assert!(self.message.is_some()); // Check expected to be called first
				swap.message2 = Some(self.message.clone().unwrap());

				Ok(StateProcessRespond::new(
					StateId::BuyerWaitingForRespondRedeemMessage,
				))
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerSendingInitRedeemMessage get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitingForLockConfirmations)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitingForRespondRedeemMessage)
	}
}

///////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerWaitingForRespondRedeemMessage
pub struct BuyerWaitingForRespondRedeemMessage<K: Keychain> {
	keychain: Arc<K>,
}
impl<K: Keychain> BuyerWaitingForRespondRedeemMessage<K> {
	/// Create new instance
	pub fn new(keychain: Arc<K>) -> Self {
		Self { keychain }
	}
}
impl<K: Keychain> State for BuyerWaitingForRespondRedeemMessage<K> {
	fn get_state_id(&self) -> StateId {
		StateId::BuyerWaitingForRespondRedeemMessage
	}
	fn get_name(&self) -> String {
		"Waiting For Redeem respond message from the Seller".to_string()
	}
	fn is_cancellable(&self) -> bool {
		true
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		context: &Context,
		tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Cancel => Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime)),
			Input::Check => {
				// Check first if everything is still locked...
				let mwc_lock = tx_conf.mwc_lock_conf.unwrap_or(0);
				let secondary_lock = tx_conf.secondary_lock_conf.unwrap_or(0);
				if mwc_lock < swap.mwc_confirmations
					|| secondary_lock < swap.secondary_confirmations
				{
					return Ok(StateProcessRespond::new(
						StateId::BuyerWaitingForLockConfirmations,
					));
				}

				if swap
					.refund_slate
					.tx
					.validate(
						Weighting::AsTransaction,
						Arc::new(RwLock::new(LruVerifierCache::new())),
					)
					.is_ok()
				{
					// Was already processed. Can go to the next step
					return Ok(StateProcessRespond::new(StateId::BuyerRedeemMwc));
				}

				let time_limit = swap.get_time_message_redeem();
				if swap::get_cur_time() < time_limit {
					Ok(
						StateProcessRespond::new(StateId::BuyerWaitingForRespondRedeemMessage)
							.action(Action::BuyerWaitingForRedeemMessage)
							.time_limit(time_limit),
					)
				} else {
					// cancelling
					Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime))
				}
			}
			Input::IncomeMessage(message) => {
				debug_assert!(swap
					.redeem_slate
					.tx
					.validate(
						Weighting::AsTransaction,
						Arc::new(RwLock::new(LruVerifierCache::new()))
					)
					.is_err());

				let (_, redeem, _) = message.unwrap_redeem()?;
				BuyApi::finalize_redeem_slate(
					&*self.keychain,
					swap,
					context,
					redeem.redeem_participant,
				)?;

				debug_assert!(swap
					.redeem_slate
					.tx
					.validate(
						Weighting::AsTransaction,
						Arc::new(RwLock::new(LruVerifierCache::new()))
					)
					.is_ok());
				Ok(StateProcessRespond::new(StateId::BuyerRedeemMwc))
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerWaitingForRespondRedeemMessage get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerSendingInitRedeemMessage)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerRedeemMwc)
	}
}

////////////////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerRedeemMwc
pub struct BuyerRedeemMwc<'a, C, B>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
{
	swap_api: Arc<BtcSwapApi<'a, C, B>>,
	phantom: PhantomData<&'a C>,
}

impl<'a, C, B> BuyerRedeemMwc<'a, C, B>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
{
	/// Create a new instance
	pub fn new(swap_api: Arc<BtcSwapApi<'a, C, B>>) -> Self {
		Self {
			swap_api,
			phantom: PhantomData,
		}
	}
}

impl<'a, C, B> State for BuyerRedeemMwc<'a, C, B>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
{
	fn get_state_id(&self) -> StateId {
		StateId::BuyerRedeemMwc
	}
	fn get_name(&self) -> String {
		"Buyer redeem MWC".to_string()
	}
	fn is_cancellable(&self) -> bool {
		true
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		_context: &Context,
		tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		let time_limit = swap.get_time_mwc_redeem();
		match input {
			Input::Cancel => Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime)),
			Input::Check => {
				// Redeem slate is already published, can go forward
				if tx_conf.mwc_redeem_conf.unwrap_or(0) > 0 {
					return Ok(StateProcessRespond::new(
						StateId::BuyerWaitForRedeemMwcConfirmations,
					));
				}

				// Sorry, too late to redeem, risk is higher then expected
				if swap::get_cur_time() > time_limit {
					return Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime));
				}

				// Check if everything is still locked...
				let mwc_lock = tx_conf.mwc_lock_conf.unwrap_or(0);
				let secondary_lock = tx_conf.secondary_lock_conf.unwrap_or(0);
				if mwc_lock < swap.mwc_confirmations
					|| secondary_lock < swap.secondary_confirmations
				{
					return Ok(StateProcessRespond::new(
						StateId::BuyerWaitingForLockConfirmations,
					));
				}

				// Still waiting...
				Ok(StateProcessRespond::new(StateId::BuyerRedeemMwc)
					.action(Action::BuyerPublishMwcRedeemTx)
					.time_limit(time_limit))
			}
			Input::Execute {
				refund_address: _,
				fee_satoshi_per_byte: _,
			} => {
				if swap::get_cur_time() > time_limit {
					// too late, exiting
					return Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime));
				}

				// Check if everything is still locked...
				let mwc_lock = tx_conf.mwc_lock_conf.unwrap_or(0);
				let secondary_lock = tx_conf.secondary_lock_conf.unwrap_or(0);
				if mwc_lock < swap.mwc_confirmations
					|| secondary_lock < swap.secondary_confirmations
				{
					return Ok(StateProcessRespond::new(
						StateId::BuyerWaitingForLockConfirmations,
					));
				}

				publish_transaction(&*self.swap_api.node_client, &swap.redeem_slate.tx, false)?;
				swap.posted_redeem = Some(swap::get_cur_time());
				Ok(StateProcessRespond::new(
					StateId::BuyerWaitForRedeemMwcConfirmations,
				))
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerRedeemMwc get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitingForRespondRedeemMessage)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitForRedeemMwcConfirmations)
	}
}

////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerWaitForRedeemMwcConfirmations
pub struct BuyerWaitForRedeemMwcConfirmations {}
impl BuyerWaitForRedeemMwcConfirmations {
	/// Create new instance
	pub fn new() -> Self {
		Self {}
	}
}

impl State for BuyerWaitForRedeemMwcConfirmations {
	fn get_state_id(&self) -> StateId {
		StateId::BuyerWaitForRedeemMwcConfirmations
	}
	fn get_name(&self) -> String {
		"Waiting For Redeem Tx Confirmations".to_string()
	}
	fn is_cancellable(&self) -> bool {
		false
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		_context: &Context,
		tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Check => {
				// Check the deadline for locking
				// TODO   Check if need to do a retry.

				let conf = tx_conf.mwc_redeem_conf.unwrap_or(0);
				if conf >= swap.secondary_confirmations {
					// We are done
					return Ok(StateProcessRespond::new(StateId::BuyerSwapComplete));
				}

				if tx_conf.mwc_redeem_conf.is_none()
					&& swap.posted_redeem.unwrap_or(0)
						< swap::get_cur_time() - super::state::POST_MWC_RETRY_PERIOD
				{
					// We can retry to post
					return Ok(StateProcessRespond::new(StateId::BuyerRedeemMwc));
				}

				return Ok(
					StateProcessRespond::new(StateId::BuyerWaitForRedeemMwcConfirmations).action(
						Action::WaitForMwcConfirmations {
							name: "Redeem Transaction".to_string(),
							required: swap.mwc_confirmations,
							actual: conf,
						},
					),
				);
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerWaitForRedeemMwcConfirmations get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerRedeemMwc)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerSwapComplete)
	}
}

///////////////////////////////////////////////////////////////////////////////////////////////////

/////////////////////////////////////////////////////////////////////////////////

/// State BuyerSwapComplete
pub struct BuyerSwapComplete {}
impl BuyerSwapComplete {
	/// Create new instance
	pub fn new() -> Self {
		Self {}
	}
}
impl State for BuyerSwapComplete {
	fn get_state_id(&self) -> StateId {
		StateId::BuyerSwapComplete
	}
	fn get_name(&self) -> String {
		"Swap is completed sucessufully".to_string()
	}
	fn is_cancellable(&self) -> bool {
		false
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		_swap: &mut Swap,
		_context: &Context,
		_tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Check => Ok(StateProcessRespond::new(StateId::BuyerSwapComplete)),
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerSwapComplete get {:?}",
				input
			))),
		}
	}

	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitForRedeemMwcConfirmations)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		None
	}
}

///////////////////////////////////////////////////////////////////

/// State BuyerCancelled
pub struct BuyerCancelled {}
impl BuyerCancelled {
	/// Create new instance
	pub fn new() -> Self {
		Self {}
	}
}
impl State for BuyerCancelled {
	fn get_state_id(&self) -> StateId {
		StateId::BuyerCancelled
	}
	fn get_name(&self) -> String {
		"Swap is cancelled, no funds was locked, no refund needed".to_string()
	}
	fn is_cancellable(&self) -> bool {
		false
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		_swap: &mut Swap,
		_context: &Context,
		_tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Check => Ok(StateProcessRespond::new(StateId::BuyerCancelled)),
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerCancelled get {:?}",
				input
			))),
		}
	}

	fn get_prev_swap_state(&self) -> Option<StateId> {
		None
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		None
	}
}

/////////////////////////////////////////////////////////////////////////////////////////////////////
//     Refund workflow
////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerWaitingForRefundTime
pub struct BuyerWaitingForRefundTime {}

impl BuyerWaitingForRefundTime {
	/// Create a new instance
	pub fn new() -> Self {
		Self {}
	}
}

impl State for BuyerWaitingForRefundTime {
	fn get_state_id(&self) -> StateId {
		StateId::BuyerWaitingForRefundTime
	}
	fn get_name(&self) -> String {
		"Waiting for Secondary to unlock".to_string()
	}
	fn is_cancellable(&self) -> bool {
		false
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		_context: &Context,
		tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Check => {
				// Just chilling. MWC redeem was never posted, so Seller can't get BTC. But still checking for what if

				// Should be impossible scenarion. But somehow the slate was posted. In any case we
				// have nothing loose, that slate is ours in any case.
				debug_assert!(tx_conf.mwc_redeem_conf.is_none());

				let cur_time = swap::get_cur_time();
				let time_limit = swap.get_time_btc_lock();
				if cur_time > time_limit {
					return Ok(StateProcessRespond::new(
						StateId::BuyerPostingRefundForSecondary,
					));
				}

				// Still waiting...
				Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime)
					.action(Action::WaitingForBtcRefund {
						currency: swap.secondary_currency,
						required: time_limit as u64,
						current: cur_time as u64,
					})
					.time_limit(time_limit))
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerWaitingForRefundTime get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		None
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerPostingRefundForSecondary)
	}
}

/////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerPostingRefundForSecondary
pub struct BuyerPostingRefundForSecondary<'a, C, B, K>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
	K: Keychain + 'a,
{
	keychain: Arc<K>,
	swap_api: Arc<BtcSwapApi<'a, C, B>>,
	phantom: PhantomData<&'a C>,
}
impl<'a, C, B, K> BuyerPostingRefundForSecondary<'a, C, B, K>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
	K: Keychain + 'a,
{
	/// Create new instance
	pub fn new(keychain: Arc<K>, swap_api: Arc<BtcSwapApi<'a, C, B>>) -> Self {
		Self {
			keychain,
			swap_api,
			phantom: PhantomData,
		}
	}
}

impl<'a, C, B, K> State for BuyerPostingRefundForSecondary<'a, C, B, K>
where
	C: NodeClient + 'a,
	B: BtcNodeClient + 'a,
	K: Keychain + 'a,
{
	fn get_state_id(&self) -> StateId {
		StateId::BuyerPostingRefundForSecondary
	}
	fn get_name(&self) -> String {
		"Posting Refund for Secondary".to_string()
	}
	fn is_cancellable(&self) -> bool {
		false
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		context: &Context,
		tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Check => {
				let cur_time = swap::get_cur_time();
				let time_limit = swap.get_time_btc_lock();
				if cur_time < time_limit {
					return Ok(StateProcessRespond::new(StateId::BuyerWaitingForRefundTime));
				}

				// Check if refund is already issued
				if tx_conf.secondary_refund_conf.is_some() {
					return Ok(StateProcessRespond::new(
						StateId::BuyerWaitingForRefundConfirmations,
					));
				}

				Ok(
					StateProcessRespond::new(StateId::BuyerPostingRefundForSecondary).action(
						Action::BuyerPublishSecondaryRefundTx(swap.secondary_currency),
					),
				)
			}
			Input::Execute {
				refund_address,
				fee_satoshi_per_byte,
			} => {
				self.swap_api.post_secondary_refund_tx(
					&*self.keychain,
					context,
					swap,
					refund_address.clone(),
					fee_satoshi_per_byte.clone(),
				)?;
				swap.posted_refund = Some(swap::get_cur_time());
				Ok(StateProcessRespond::new(
					StateId::BuyerWaitingForRefundConfirmations,
				))
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerPostingRefundForSecondary get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitingForRefundTime)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitingForRefundConfirmations)
	}
}

/////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerWaitingForRefundConfirmations
pub struct BuyerWaitingForRefundConfirmations {}
impl BuyerWaitingForRefundConfirmations {
	/// Create new instance
	pub fn new() -> Self {
		Self {}
	}
}

impl State for BuyerWaitingForRefundConfirmations {
	fn get_state_id(&self) -> StateId {
		StateId::BuyerWaitingForRefundConfirmations
	}
	fn get_name(&self) -> String {
		"Waiting for Refund confirmations".to_string()
	}
	fn is_cancellable(&self) -> bool {
		false
	}

	/// Process the state. Result will be the next state
	fn process(
		&mut self,
		input: Input,
		swap: &mut Swap,
		_context: &Context,
		tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Check => {
				if let Some(conf) = tx_conf.secondary_refund_conf {
					if conf >= swap.secondary_confirmations {
						// We are done
						return Ok(StateProcessRespond::new(StateId::BuyerCancelledRefunded));
					}
				} else {
					// might need to retry
					if swap.posted_refund.unwrap_or(0)
						< swap::get_cur_time() - super::state::POST_SECONDARY_RETRY_PERIOD
					{
						return Ok(StateProcessRespond::new(
							StateId::BuyerPostingRefundForSecondary,
						));
					}
				}

				Ok(
					StateProcessRespond::new(StateId::BuyerWaitingForRefundConfirmations).action(
						Action::WaitForSecondaryConfirmations {
							name: format!("{} Refund", swap.secondary_currency),
							currency: swap.secondary_currency,
							required: swap.secondary_confirmations,
							actual: tx_conf.secondary_refund_conf.unwrap_or(0),
						},
					),
				)
			}
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerWaitingForRefundConfirmations get {:?}",
				input
			))),
		}
	}
	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerPostingRefundForSecondary)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerCancelledRefunded)
	}
}

//////////////////////////////////////////////////////////////////////////////////////////////////////

/// State BuyerCancelledRefunded
pub struct BuyerCancelledRefunded {}
impl BuyerCancelledRefunded {
	/// Create new instance
	pub fn new() -> Self {
		Self {}
	}
}
impl State for BuyerCancelledRefunded {
	fn get_state_id(&self) -> StateId {
		StateId::BuyerCancelledRefunded
	}
	fn get_name(&self) -> String {
		"Swap is cancelled, refund is redeemed".to_string()
	}
	fn is_cancellable(&self) -> bool {
		false
	}

	fn process(
		&mut self,
		input: Input,
		_swap: &mut Swap,
		_context: &Context,
		_tx_conf: &SwapTransactionsConfirmations,
	) -> Result<StateProcessRespond, ErrorKind> {
		match input {
			Input::Check => Ok(StateProcessRespond::new(StateId::BuyerCancelledRefunded)),
			_ => Err(ErrorKind::InvalidSwapStateInput(format!(
				"BuyerCancelledRefunded get {:?}",
				input
			))),
		}
	}

	fn get_prev_swap_state(&self) -> Option<StateId> {
		Some(StateId::BuyerWaitingForRefundConfirmations)
	}
	fn get_next_swap_state(&self) -> Option<StateId> {
		None
	}
}
