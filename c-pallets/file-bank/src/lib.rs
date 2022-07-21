//! # File Bank Module
//!
//! Contain operations related info of files on multi-direction.
//!
//! ### Terminology
//!
//! * **Is Public:** Public or private.
//! * **Backups:** Number of duplicate.
//! * **Deadline:** Expiration time.
//!
//!
//! ### Interface
//!
//! ### Dispatchable Functions
//!
//! * `upload` - Upload info of stored file.
//! * `update` - Update info of uploaded file.
//! * `buyfile` - Buy file with download fee.
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

use frame_support::traits::{
	Currency, ExistenceRequirement::AllowDeath, FindAuthor, Randomness, ReservableCurrency,
};
pub use pallet::*;
#[cfg(feature = "runtime-benchmarks")]
pub mod benchmarking;
pub mod weights;

mod types;
pub use types::*;

use codec::{Decode, Encode};
use scale_info::TypeInfo;
use sp_runtime::{
	offchain as rt_offchain,
	traits::{
		AccountIdConversion, BlockNumberProvider, CheckedAdd, CheckedDiv, CheckedMul, CheckedSub,
		SaturatedConversion,
	},
	RuntimeDebug,
};
use sp_std::{convert::TryInto, prelude::*, str};

use frame_support::{dispatch::DispatchResult, pallet_prelude::*, PalletId};
use frame_system::offchain::{AppCrypto, CreateSignedTransaction, SendSignedTransaction, Signer};
use sp_core::crypto::KeyTypeId;
pub use weights::WeightInfo;

type AccountOf<T> = <T as frame_system::Config>::AccountId;
type BalanceOf<T> =
	<<T as pallet::Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;
type BlockNumberOf<T> = <T as frame_system::Config>::BlockNumber;
type BoundedString<T> = BoundedVec<u8, <T as Config>::StringLimit>;
type BoundedList<T> =
	BoundedVec<BoundedVec<u8, <T as Config>::StringLimit>, <T as Config>::StringLimit>;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::{ensure, traits::Get, inherent::BlockT};
	use pallet_file_map::ScheduleFind;
	use pallet_sminer::MinerControl;
	//pub use crate::weights::WeightInfo;
	use frame_system::{ensure_signed, pallet_prelude::*};

	const HTTP_REQUEST_STR: &str = "https://arweave.net/price/1048576";
	// const HTTP_REQUEST_STR: &str = "https://api.coincap.io/v2/assets/polkadot";
	pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"cess");
	const FETCH_TIMEOUT_PERIOD: u64 = 60_000; // in milli-seconds
										  //1MB converted byte size
	const M_BYTE: u128 = 1_048_576;
	const G_BYTE: u128 = 1_048_576 * 1024;
	const T_BYTE: u128 = 1_048_576 * 1024 * 1024;

	pub mod crypto {
		use super::KEY_TYPE;
		use sp_core::sr25519::Signature as Sr25519Signature;
		use sp_runtime::{
			app_crypto::{app_crypto, sr25519},
			traits::Verify,
			MultiSignature, MultiSigner,
		};

		app_crypto!(sr25519, KEY_TYPE);

		pub struct TestAuthId;
		// implemented for ocw-runtime
		impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
			type RuntimeAppPublic = Public;
			type GenericSignature = sp_core::sr25519::Signature;
			type GenericPublic = sp_core::sr25519::Public;
		}

		// implemented for mock runtime in test
		impl
			frame_system::offchain::AppCrypto<
				<Sr25519Signature as Verify>::Signer,
				Sr25519Signature,
			> for TestAuthId
		{
			type RuntimeAppPublic = Public;
			type GenericSignature = sp_core::sr25519::Signature;
			type GenericPublic = sp_core::sr25519::Public;
		}
	}

	#[pallet::config]
	pub trait Config:
		frame_system::Config
		+ pallet_sminer::Config
		+ sp_std::fmt::Debug
		+ CreateSignedTransaction<Call<Self>>
	{
		/// The overarching event type.
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
		/// The currency trait.
		type Currency: ReservableCurrency<Self::AccountId>;

		type WeightInfo: WeightInfo;

		type Call: From<Call<Self>>;

		//Find the consensus of the current block
		type FindAuthor: FindAuthor<Self::AccountId>;

		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;
		//Used to find out whether the schedule exists
		type Scheduler: ScheduleFind<Self::AccountId>;
		//It is used to control the computing power and space of miners
		type MinerControl: MinerControl<Self::AccountId>;
		//Interface that can generate random seeds
		type MyRandomness: Randomness<Self::Hash, Self::BlockNumber>;
		/// pallet address.
		#[pallet::constant]
		type FilbakPalletId: Get<PalletId>;

		#[pallet::constant]
		type StringLimit: Get<u32> + Clone + Eq + PartialEq;

		#[pallet::constant]
		type OneDay: Get<BlockNumberOf<Self>>;
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		//file upload declaration
		UploadDeclaration { acc: AccountOf<T>, file_hash: Vec<u8>, file_name: Vec<u8> },
		//file uploaded.
		FileUpload { acc: AccountOf<T> },
		//file updated.
		FileUpdate { acc: AccountOf<T>, fileid: Vec<u8> },

		FileChangeState { acc: AccountOf<T>, fileid: Vec<u8> },
		//file bought.
		BuyFile { acc: AccountOf<T>, money: BalanceOf<T>, fileid: Vec<u8> },
		//file purchased before.
		Purchased { acc: AccountOf<T>, fileid: Vec<u8> },
		//Storage information of scheduling storage file slice
		InsertFileSlice { fileid: Vec<u8> },
		//User purchase space
		BuyPackage { acc: AccountOf<T>, size: u128, fee: BalanceOf<T> },
		//Expired storage space
		LeaseExpired { acc: AccountOf<T>, size: u128 },
		//Storage space expiring within 24 hours
		LeaseExpireIn24Hours { acc: AccountOf<T>, size: u128 },
		//File deletion event
		DeleteFile { acc: AccountOf<T>, fileid: Vec<u8> },
		//Filler chain success event
		FillerUpload { acc: AccountOf<T>, file_size: u64 },
		//File recovery
		RecoverFile { acc: AccountOf<T>, file_hash: Vec<u8> },
		//The miner cleaned up an invalid file event
		ClearInvalidFile { acc: AccountOf<T>, file_hash: Vec<u8> },
		//Users receive free space events
		ReceiveSpace { acc: AccountOf<T> },
	}
	#[pallet::error]
	pub enum Error<T> {
		FileExistent,
		//file doesn't exist.
		FileNonExistent,
		//overflow.
		Overflow,
		//When the user uploads a file, the purchased space is not enough
		InsufficientStorage,
		//Internal developer usage error
		WrongOperation,
		//haven't bought space at all
		NotPurchasedPackage,

		PurchasedPackage,
		//Expired storage space
		LeaseExpired,

		LeaseFreeze,
		//Exceeded the maximum amount expected by the user
		ExceedExpectations,

		ConversionError,

		InsufficientAvailableSpace,

		AlreadyRepair,

		NotOwner,

		AlreadyReceive,

		AlreadyExist,

		NotQualified,

		UserNotDeclared,
		//Signature error of offline working machine
		NoLocalAcctForSigning,
		//It is not an error message for scheduling operation
		ScheduleNonExistent,
		//Error reporting when boundedvec is converted to VEC
		BoundedVecError,
		//Error that the storage has reached the upper limit.
		StorageLimitReached,
		//The miner's calculation power is insufficient, resulting in an error that cannot be
		// replaced
		MinerPowerInsufficient,

		IsZero,
		//Multi consensus query restriction of off chain workers
		Locked,

		LengthExceedsLimit,

		Declarated,
	}
	#[pallet::storage]
	#[pallet::getter(fn next_unsigned_at)]
	pub(super) type NextUnsignedAt<T: Config> = StorageValue<_, T::BlockNumber, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn file)]
	pub(super) type File<T: Config> =
		StorageMap<_, Blake2_128Concat, BoundedString<T>, FileInfo<T>>;

	#[pallet::storage]
	#[pallet::getter(fn invoice)]
	pub(super) type Invoice<T: Config> =
		StorageMap<_, Blake2_128Concat, BoundedString<T>, u8, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn user_hold_file_list)]
	pub(super) type UserHoldFileList<T: Config> =
		StorageMap<
			_, 
			Blake2_128Concat, 
			T::AccountId, 
			BoundedVec<UserFileSliceInfo<T>, T::ItemLimit>, 
			ValueQuery
		>;

	#[pallet::storage]
	pub(super) type UserFreeRecord<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, u8, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn unit_price)]
	pub(super) type UnitPrice<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn filler_map)]
	pub(super) type FillerMap<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		AccountOf<T>,
		Blake2_128Concat,
		BoundedString<T>,
		FillerInfo<T>,
	>;

	#[pallet::storage]
	#[pallet::getter(fn invalid_file)]
	pub(super) type InvalidFile<T: Config> =
		StorageMap<_, Blake2_128Concat, AccountOf<T>, BoundedList<T>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn members)]
	pub(super) type Members<T: Config> =
		StorageValue<_, BoundedVec<AccountOf<T>, T::StringLimit>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn lock_time)]
	pub(super) type LockTime<T: Config> = StorageValue<_, BlockNumberOf<T>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn purchase_package)]
	pub(super) type PurchasedPackage<T: Config> = StorageMap<_, Blake2_128Concat, AccountOf<T>, PackageDetails<T>>;

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(PhantomData<T>);

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberOf<T>> for Pallet<T> {
		//Used to calculate whether it is implied to submit spatiotemporal proof
		//Cycle every 7.2 hours
		//When there is an uncommitted space-time certificate, the corresponding miner will be
		// punished and the corresponding data segment will be removed
		fn on_initialize(now: BlockNumberOf<T>) -> Weight {
			let number: u128 = now.saturated_into();
			let block_oneday: BlockNumberOf<T> = T::OneDay::get();
			let oneday: u32 = block_oneday.saturated_into();
			let mut count: u8 = 0;
			if number % oneday as u128 == 0 {
				log::info!("Start lease expiration check");
				for (acc, info) in <PurchasedPackage<T>>::iter() {
					if info.deadline > now {
						let frozen_day: BlockNumberOf<T> = match info.package_type {
							1 => (0 * oneday).saturated_into(),
							2 => (7 * oneday).saturated_into(),
							3 => (14 * oneday).saturated_into(),
							4 => (20 * oneday).saturated_into(),
							5 => (30 * oneday).saturated_into(),
						};
						if info.deadline + frozen_day > now {
							Self::
						}

						let result = <PurchasedPackage<T>>::try_mutate(&acc, |s_opt| -> DispatchResult {

							Ok(())
						})?;
					}
				}
			}
			0
		}

	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::weight(6_231_000)]
		pub fn upload_declaration(
			origin: OriginFor<T>,
			file_hash: Vec<u8>,
			file_name: Vec<u8>,
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;
			let file_hash_bound: BoundedString<T> = file_hash.clone().try_into().map_err(|_| Error::<T>::Overflow)?;
			let file_name_bound: BoundedString<T> = file_name.clone().try_into().map_err(|_| Error::<T>::Overflow)?;
			if <File<T>>::contains_key(&file_hash_bound) {
				<File<T>>::try_mutate(&file_hash_bound, |s_opt| -> DispatchResult {
					let s = s_opt.as_mut().ok_or(Error::<T>::FileNonExistent)?;
					if s.user.contains(&sender) {
						Err(Error::<T>::Declarated)?;
					}
					Self::update_user_space(sender.clone(), 1, s.file_size.into())?;
					Self::add_user_hold_fileslice(sender.clone(), file_hash_bound.clone(), s.file_size)?;
					s.user.try_push(sender.clone()).map_err(|_| Error::<T>::StorageLimitReached)?;
					s.file_name.try_push(file_name_bound.clone()).map_err(|_| Error::<T>::StorageLimitReached)?;
					Ok(())
				})?;
			} else {
				<File<T>>::insert(
					&file_hash_bound,
					FileInfo::<T>{
						file_size: 0,
						block_num: 0,		
						scan_size: 0,			
						segment_size: 0,	
						miner_acc: sender.clone(),
						miner_id: 0,
						miner_ip: Default::default(),
						user: vec![sender.clone()].try_into().map_err(|_| Error::<T>::BoundedVecError)?,
						file_name: vec![file_name_bound].try_into().map_err(|_| Error::<T>::BoundedVecError)?,
						file_state: "pending".as_bytes().to_vec().try_into().map_err(|_| Error::<T>::BoundedVecError)?,
					},
				);
			}
			Self::deposit_event(Event::<T>::UploadDeclaration { acc: sender, file_hash: file_hash, file_name: file_name });
			Ok(())
		}
		/// Upload info of stored file.
		///
		/// The dispatch origin of this call must be _Signed_.
		#[pallet::weight(<T as pallet::Config>::WeightInfo::upload())]
		pub fn upload(
			origin: OriginFor<T>,
			file_hash: Vec<u8>,
			file_size: u64,
			block_num: u32,
			scan_size: u32,
			segment_size: u32,
			miner_acc: AccountOf<T>,
			miner_id: u64,
			miner_ip: Vec<u8>,
			user: AccountOf<T>,
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;
			if !T::Scheduler::contains_scheduler(sender.clone()) {
				Err(Error::<T>::ScheduleNonExistent)?;
			}
			let file_hash_bounded: BoundedString<T> = file_hash.try_into().map_err(|_| Error::<T>::BoundedVecError)?;
			if !<File<T>>::contains_key(&file_hash_bounded) {
				Err(Error::<T>::FileNonExistent)?;
			}
			Self::update_user_space(user.clone(), 1, file_size.into())?;

			<File<T>>::try_mutate(&file_hash_bounded, |s_opt| -> DispatchResult {
				let s = s_opt.as_mut().unwrap();
				if !s.user.contains(&user) {
					Err(Error::<T>::UserNotDeclared)?;
				}
				if s.file_state.to_vec() == "active".as_bytes().to_vec() {
					Err(Error::<T>::FileExistent)?;
				}
				s.file_size = file_size;
				s.block_num = block_num;
				s.scan_size = scan_size;
				s.segment_size = segment_size;
				s.miner_acc = miner_acc.clone();
				s.miner_id = miner_id;
				s.miner_ip = miner_ip.try_into().map_err(|_| Error::<T>::BoundedVecError)?;
				s.file_state = "active".as_bytes().to_vec().try_into().map_err(|_| Error::<T>::BoundedVecError)?;
				Ok(())
			})?;

			Self::add_user_hold_fileslice(user.clone() ,file_hash_bounded.clone(), file_size)?;

			Self::replace_file(miner_acc.clone(), file_size)?;

			Self::deposit_event(Event::<T>::FileUpload { acc: user.clone() });
			Ok(())
		}

		//The filler upload interface can only be called by scheduling, and the list has a maximum
		// length limit
		#[pallet::weight(1_000)]
		pub fn upload_filler(
			origin: OriginFor<T>,
			miner: AccountOf<T>,
			filler_list: Vec<FillerInfo<T>>,
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;
			if filler_list.len() > 10 {
				Err(Error::<T>::LengthExceedsLimit)?;
			}
			if !T::Scheduler::contains_scheduler(sender.clone()) {
				Err(Error::<T>::ScheduleNonExistent)?;
			}
			let miner_state = T::MinerControl::get_miner_state(miner.clone())?;
			if !(miner_state == "positive".as_bytes().to_vec()) {
				Err(Error::<T>::NotQualified)?;
			}
			for i in filler_list.iter() {
				if <FillerMap<T>>::contains_key(&miner, i.filler_id.clone()) {
					Err(Error::<T>::FileExistent)?;
				}
				<FillerMap<T>>::insert(miner.clone(), i.filler_id.clone(), i);
			}

			let power = M_BYTE
				.checked_mul(8)
				.ok_or(Error::<T>::Overflow)?
				.checked_mul(filler_list.len() as u128)
				.ok_or(Error::<T>::Overflow)?;
			T::MinerControl::add_power(miner.clone(), power)?;
			Self::deposit_event(Event::<T>::FillerUpload { acc: sender, file_size: power as u64 });
			Ok(())
		}

		#[pallet::weight(2_000_000)]
		pub fn delete_file(origin: OriginFor<T>, fileid: Vec<u8>) -> DispatchResult {
			let sender = ensure_signed(origin)?;
			let bounded_fileid = Self::vec_to_bound::<u8>(fileid.clone())?;
			ensure!(<File<T>>::contains_key(bounded_fileid.clone()), Error::<T>::FileNonExistent);
			//The above has been judged. Unwrap will be performed only if the key exists
			Self::clear_user_file(bounded_fileid, &sender)?;

			Self::deposit_event(Event::<T>::DeleteFile { acc: sender, fileid });
			Ok(())
		}

		//**********************************************************************************************************************************************
		//************************************************************Storage space lease***********
		//************************************************************Storage **********************
		//************************************************************Storage **********************
		//************************************************************Storage ********
		//**********************************************************************************************************************************************
		//The parameter "space_count" is calculated in gigabyte.
		//parameter "lease_count" is calculated on the monthly basis.
		#[pallet::weight(2_000_000)]
		pub fn buy_package(
			origin: OriginFor<T>,
			package_type: u8,
			count: u128,
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;

			ensure!(<PurchasedPackage<T>>::contains_key(&sender), Error::<T>::PurchasedPackage);

			let (space, m_unit_price, month) = match package_type {
				1 => (10 * G_BYTE, 0, 1),
				2 => (500 * G_BYTE ,Self::get_price(500 * G_BYTE)?, 1),
				3 => (T_BYTE, Self::get_price(T_BYTE)?, 1),
				4 => (T_BYTE, Self::get_price(5 * T_BYTE)?, 1),
				5 => {
					if count < 5 {
						 Err(Error::<T>::WrongOperation)?;
					}
					(count * T_BYTE, Self::get_price(count * T_BYTE)?, 1)
				},
				_ => Err(Error::<T>::WrongOperation)?,
			};

			Self::add_puchased_package(sender.clone(), space, month as u32, package_type)?;

			let g_unit_price = m_unit_price.checked_div(G_BYTE).ok_or(Error::<T>::Overflow)?;
			let price: BalanceOf<T> = space
				.checked_div(G_BYTE)
				.ok_or(Error::<T>::Overflow)?
				.checked_mul(g_unit_price)
				.ok_or(Error::<T>::Overflow)?
				.try_into()
				.map_err(|_e| Error::<T>::Overflow)?;

			let acc = T::FilbakPalletId::get().into_account();
			<T as pallet::Config>::Currency::transfer(&sender, &acc, price, AllowDeath)?;
			
			Ok(())
		}

		//Feedback results after the miner clears the invalid files
		#[pallet::weight(10_000)]
		pub fn clear_invalid_file(
			origin: OriginFor<T>,
			file_hash: Vec<u8>,
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;
			let bounded_string: BoundedString<T> =
				file_hash.clone().try_into().map_err(|_e| Error::<T>::BoundedVecError)?;
			<InvalidFile<T>>::try_mutate(&sender, |o| -> DispatchResult {
				o.retain(|x| *x != bounded_string);
				Ok(())
			})?;
			Self::deposit_event(Event::<T>::ClearInvalidFile { acc: sender, file_hash });
			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn add_member(origin: OriginFor<T>, acc: AccountOf<T>) -> DispatchResult {
			let _ = ensure_root(origin)?;
			let member_list = Self::members();
			if member_list.contains(&acc) {
				Err(Error::<T>::AlreadyExist)?;
			}
			<Members<T>>::try_mutate(|o| -> DispatchResult {
				o.try_push(acc).map_err(|_e| Error::<T>::StorageLimitReached)?;
				Ok(())
			})?;
			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn del_member(origin: OriginFor<T>, acc: AccountOf<T>) -> DispatchResult {
			let _ = ensure_root(origin)?;
			<Members<T>>::try_mutate(|o| -> DispatchResult {
				o.retain(|x| x != &acc);
				Ok(())
			})?;
			Ok(())
		}

		#[pallet::weight(10_000)]
		pub fn clear_all_filler(origin: OriginFor<T>) -> DispatchResult {
			let sender = ensure_signed(origin)?;
			let state = T::MinerControl::get_miner_state(sender.clone())?;
			if state != "exit".as_bytes().to_vec() {
				Err(Error::<T>::NotQualified)?;
			}
			let _ = FillerMap::<T>::remove_prefix(&sender, Option::None);
			Ok(())
		} 
	}

	impl<T: Config> Pallet<T> {
		fn add_puchased_package(acc: AccountOf<T>, space: u128, month: u32, package_type: u8) -> DispatchResult {
			let now = <frame_system::Pallet<T>>::block_number();
			let sur_block: BlockNumberOf<T> = month
				.checked_mul(30)
				.ok_or(Error::<T>::Overflow)?
				.checked_mul(86400)
				.ok_or(Error::<T>::Overflow)?
				.checked_div(3)
				.ok_or(Error::<T>::Overflow)?
				.saturated_into();
			let deadline = now.checked_add(&sur_block).ok_or(Error::<T>::Overflow)?;
			let info = PackageDetails::<T>{
				space: space,
				used_space: 0,
				remaining_space: 0,
				tenancy: month,
				package_type: package_type,
				start: now,
				deadline: deadline,
				state: "normal".as_bytes().to_vec().try_into().map_err(|_e| Error::<T>::BoundedVecError)?,
			};

			<PurchasedPackage<T>>::insert(&acc, info);
			Ok(())
		}

		//operation: 1 upload files, 2 delete file
		fn update_user_space(acc: AccountOf<T>, operation: u8, size: u128) -> DispatchResult {
			match operation {
				1 => {
						<PurchasedPackage<T>>::try_mutate(&acc, |s_opt| -> DispatchResult {
							let s = s_opt.as_mut().ok_or(Error::<T>::NotPurchasedPackage)?;
							if s.state.to_vec() == "frozen".as_bytes().to_vec() {
								Err(Error::<T>::LeaseFreeze)?;
							}
							if size > s.space - s.used_space {
								Err(Error::<T>::InsufficientStorage)?;
							}
							s.used_space = s.used_space.checked_add(size).ok_or(Error::<T>::Overflow)?;
							s.remaining_space =
								s.remaining_space.checked_sub(size).ok_or(Error::<T>::Overflow)?;
							Ok(())
						})?;
				}
				2 => <PurchasedPackage<T>>::try_mutate(&acc, |s_opt| -> DispatchResult {
					let s = s_opt.as_mut().unwrap();
					s.used_space = s.used_space.checked_sub(size).ok_or(Error::<T>::Overflow)?;
					s.remaining_space = s.space
							.checked_sub(s.used_space)
							.ok_or(Error::<T>::Overflow)?;
					Ok(())
				})?,
				_ => Err(Error::<T>::WrongOperation)?,
			}
			Ok(())
		}

		//Available space divided by 1024 is the unit price
		fn get_price(buy_space: u128) -> Result<u128, DispatchError> {
			//Get the available space on the current chain
			let total_space = pallet_sminer::Pallet::<T>::get_space()?;
			//If it is not 0, the logic is executed normally
			if total_space == 0 {
				Err(Error::<T>::IsZero)?;
			}
			//Calculation rules
			//The price is based on 1024 / available space on the current chain
			//Multiply by the base value 1 tcess * 1_000 (1_000_000_000_000 * 1_000)
			let price: u128 = buy_space
				.checked_mul(1_000_000_000_000)
				.ok_or(Error::<T>::Overflow)?
				.checked_mul(10_000)
				.ok_or(Error::<T>::Overflow)?
				.checked_div(total_space)
				.ok_or(Error::<T>::Overflow)?
				.checked_add(1_000_000_000_000_000)
				.ok_or(Error::<T>::Overflow)?;

			return Ok(price)
		}

		fn vec_to_bound<P>(param: Vec<P>) -> Result<BoundedVec<P, T::StringLimit>, DispatchError> {
			let result: BoundedVec<P, T::StringLimit> =
				param.try_into().map_err(|_e| Error::<T>::BoundedVecError)?;
			Ok(result)
		}

		pub fn get_random_challenge_data(
		) -> Result<Vec<(AccountOf<T>, Vec<u8>, Vec<u8>, u64, u8, u32)>, DispatchError> {
			let filler_list = Self::get_random_filler()?;
			let mut data: Vec<(AccountOf<T>, Vec<u8>, Vec<u8>, u64, u8, u32)> = Vec::new();
			for v in filler_list {
				let length = v.block_num;
				let number_list = Self::get_random_numberlist(length, 1)?;
				let miner_acc = v.miner_address.clone();
				let filler_id = v.filler_id.clone().to_vec();
				let file_size = v.filler_size.clone();
				let segment_size = v.segment_size.clone();
				let mut block_list: Vec<u8> = Vec::new();
				for i in number_list.iter() {
					block_list.push(*i as u8);
				}
				data.push((miner_acc, filler_id, block_list, file_size, 1, segment_size));
			}

			let file_list = Self::get_random_file()?;
			for (file_hash, file) in file_list {
				let miner_id = T::MinerControl::get_miner_id(file.miner_acc.clone())?;
				if file.miner_id != miner_id {
					continue;
				}
				let length = file.block_num;
				let number_list = Self::get_random_numberlist(length, 1)?;
				let miner_acc = file.miner_acc.clone();
				let file_hash = file_hash.clone().to_vec();
				let file_size = file.file_size.clone();
				let segment_size = file.segment_size.clone();
				let mut block_list: Vec<u8> = Vec::new();
				for i in number_list.iter() {
					block_list.push(*i as u8);
				}
				data.push((miner_acc, file_hash, block_list, file_size, 2, segment_size));
			}

			Ok(data)
		}
		//Get random file block list
		fn get_random_filler() -> Result<Vec<FillerInfo<T>>, DispatchError> {
			let length = Self::get_fillermap_length()?;
			let number_list = Self::get_random_numberlist(length, 1)?;
			let mut filler_list: Vec<FillerInfo<T>> = Vec::new();
			for i in number_list.iter() {
				let mut counter: u32 = 0;
				for (_, _, value) in <FillerMap<T>>::iter() {
					if counter == *i {
						filler_list.push(value);
						break
					}
					counter = counter.checked_add(1).ok_or(Error::<T>::Overflow)?;
				}
			}
			Ok(filler_list)
		}

		fn get_random_file() -> Result<Vec<(BoundedString<T>, FileInfo<T>)>, DispatchError> {
			let length = Self::get_file_map_length()?;
			//Extract according to the probability of 4.6% * 3
			let number_list = Self::get_random_numberlist(length, 2)?;
			let mut file_list: Vec<(BoundedString<T>, FileInfo<T>)> = Vec::new();
			for i in number_list.iter() {
				let mut counter: u32 = 0;
				for (key, value) in <File<T>>::iter() {
					if value.file_state.to_vec() == "active".as_bytes().to_vec() {
						if counter == *i {
							file_list.push(
								(
									key,
									value,
								)
							);
							break
						}
						counter = counter.checked_add(1).ok_or(Error::<T>::Overflow)?;
					}
				}
			}
			Ok(file_list)
		}

		fn get_random_numberlist(length: u32, random_type: u8) -> Result<Vec<u32>, DispatchError> {
			let mut seed: u32 = <frame_system::Pallet<T>>::block_number().saturated_into();
			if length == 0 {
				return Ok(Vec::new())
			}
			let num = match random_type {
				1 => length
					.checked_mul(46)
					.ok_or(Error::<T>::Overflow)?
					.checked_div(1000)
					.ok_or(Error::<T>::Overflow)?
					.checked_add(1)
					.ok_or(Error::<T>::Overflow)?,
				2 => length
					.checked_mul(46 * 3)
					.ok_or(Error::<T>::Overflow)?
					.checked_div(1000)
					.ok_or(Error::<T>::Overflow)?
					.checked_add(1)
					.ok_or(Error::<T>::Overflow)?,
				_ => length
					.checked_mul(46)
					.ok_or(Error::<T>::Overflow)?
					.checked_div(1000)
					.ok_or(Error::<T>::Overflow)?
					.checked_add(1)
					.ok_or(Error::<T>::Overflow)?,
			};
			let mut number_list: Vec<u32> = Vec::new();
			loop {
				seed = seed.checked_add(1).ok_or(Error::<T>::Overflow)?;
				if number_list.len() >= num as usize {
					number_list.sort();
					number_list.dedup();
					if number_list.len() >= num as usize {
						break
					}
				}
				let random = Self::generate_random_number(seed)? % length;
				log::info!("List addition: {}", random);
				number_list.push(random);
			}
			Ok(number_list)
		}

		//Get storagemap filler length
		fn get_fillermap_length() -> Result<u32, DispatchError> {
			let mut length: u32 = 0;
			for _ in <FillerMap<T>>::iter() {
				length = length.checked_add(1).ok_or(Error::<T>::Overflow)?;
			}
			Ok(length)
		}

		//Get Storage FillerMap Length
		fn get_file_map_length() -> Result<u32, DispatchError> {
			let mut length: u32 = 0;
			for (_, v) in <File<T>>::iter() {
				if v.file_state.to_vec() == "active".as_bytes().to_vec() {
					length = length.checked_add(1).ok_or(Error::<T>::Overflow)?;
				}
			}
			Ok(length)
		}

		//Get random number
		pub fn generate_random_number(seed: u32) -> Result<u32, DispatchError> {
			let mut counter = 0;
			loop {
				let (random_seed, _) =
					T::MyRandomness::random(&(T::FilbakPalletId::get(), seed + counter).encode());
				let random_number = <u32>::decode(&mut random_seed.as_ref()).unwrap_or(0);
				if random_number != 0 {
					return Ok(random_number)
				}
				counter = counter.checked_add(1).ok_or(Error::<T>::Overflow)?;
			}
		}

		//Specific implementation method of deleting filler file
		pub fn delete_filler(miner_acc: AccountOf<T>, filler_id: Vec<u8>) -> DispatchResult {
			let filler_boud: BoundedString<T> =
				filler_id.try_into().map_err(|_e| Error::<T>::BoundedVecError)?;
			if !<FillerMap<T>>::contains_key(&miner_acc, filler_boud.clone()) {
				Err(Error::<T>::FileNonExistent)?;
			}
			<FillerMap<T>>::remove(miner_acc, filler_boud.clone());

			Ok(())
		}

		//Delete the next backup under the file
		pub fn clear_file(file_hash: Vec<u8>) -> DispatchResult {
			let file_hash_bounded: BoundedString<T> =
				file_hash.try_into().map_err(|_e| Error::<T>::BoundedVecError)?;
			if !<File<T>>::contains_key(&file_hash_bounded) {
				Err(Error::<T>::FileNonExistent)?;
			}
			<File<T>>::remove(file_hash_bounded);

			Ok(())
		}

		pub fn clear_user_file(file_hash: BoundedVec<u8, T::StringLimit>, user: &AccountOf<T>) -> DispatchResult {
			let file = <File<T>>::get(&file_hash).unwrap();
			ensure!(file.user.contains(user),  Error::<T>::NotOwner);
			Self::update_user_space(
				user.clone(),
				2,
				file.file_size.clone().into(),
			)?;
			//If the file still has an owner, only the corresponding owner will be cleared. 
			//If the owner is unique, the file meta information will be cleared.
			if file.user.len() > 1 {
				<File<T>>::try_mutate(&file_hash, |s_opt| -> DispatchResult {
					let s = s_opt.as_mut().unwrap();
					let mut index = 0;
					for acc in s.user.iter() {
						if *acc == user.clone() {
							break;
						}
						index = index.checked_add(&1).ok_or(Error::<T>::Overflow)?;
					}
					s.user.remove(index);
					s.file_name.remove(index);
					Ok(())
				})?;
			} else {
				<File<T>>::remove(&file_hash);
				Self::add_invalid_file(file.miner_acc.clone(), file_hash.to_vec())?;
				T::MinerControl::sub_power(file.miner_acc.clone(), file.file_size.into())?;
				T::MinerControl::sub_space(file.miner_acc.clone(), file.file_size.into())?;
			}
			
			<UserHoldFileList<T>>::try_mutate(&user, |s| -> DispatchResult {
				s.retain(|x| x.file_hash != file_hash.clone());
				Ok(())
			})?;
			Ok(())
		}

		fn replace_file(miner_acc: AccountOf<T>, file_size: u64) -> DispatchResult {
			//add space
			T::MinerControl::add_space(miner_acc.clone(), file_size.into())?;
			let (power, space) = T::MinerControl::get_power_and_space(miner_acc.clone())?;
			//Judge whether the current miner's remaining is enough to store files
			if power > space {
				if power - space < file_size.into() {
					Err(Error::<T>::MinerPowerInsufficient)?;
				}
			} else {
				Err(Error::<T>::Overflow)?;
			}
			
			//How many files to replace, round up
			let replace_num = (file_size as u128)
				.checked_div(8)
				.ok_or(Error::<T>::Overflow)?
				.checked_div(M_BYTE)
				.ok_or(Error::<T>::Overflow)?
				.checked_add(1)
				.ok_or(Error::<T>::Overflow)?;
			let mut counter = 0;
			let mut filler_id_list: BoundedList<T> = Default::default();
			for (filler_id, _) in <FillerMap<T>>::iter_prefix(miner_acc.clone()) {
				if counter == replace_num {
					break
				}
				filler_id_list.try_push(filler_id.clone()).map_err(|_| Error::<T>::StorageLimitReached)?;
				
				counter = counter.checked_add(1).ok_or(Error::<T>::Overflow)?;
				//Clear information on the chain
				Self::delete_filler(miner_acc.clone(), filler_id.to_vec())?;
			}
			
			//Notify the miner to clear the corresponding data segment
			<InvalidFile<T>>::try_mutate(&miner_acc, |o| -> DispatchResult {
				for file_hash in filler_id_list {
					o.try_push(file_hash).map_err(|_e| Error::<T>::StorageLimitReached)?;
				}
				Ok(())
			})?;

			Ok(())
		}

		//Add invalid file list, notify miner to delete
		pub fn add_invalid_file(miner_acc: AccountOf<T>, file_hash: Vec<u8>) -> DispatchResult {
			<InvalidFile<T>>::try_mutate(&miner_acc, |o| -> DispatchResult {
				o.try_push(file_hash.try_into().map_err(|_e| Error::<T>::BoundedVecError)?)
					.map_err(|_e| Error::<T>::StorageLimitReached)?;
				Ok(())
			})?;

			Ok(())
		}

		pub fn update_price_for_tests() -> DispatchResult {
			let price: BalanceOf<T> = 100u128.try_into().map_err(|_| Error::<T>::Overflow)?;
			UnitPrice::<T>::put(price);
			Ok(())
		}

		fn add_user_hold_fileslice(user: AccountOf<T>, file_hash_bound: BoundedVec<u8, T::StringLimit>, file_size: u64) -> DispatchResult {
			let file_info = UserFileSliceInfo::<T>{
				file_hash: file_hash_bound.clone(),
				file_size: file_size,
			};
			<UserHoldFileList<T>>::try_mutate(&user, |v| -> DispatchResult {
				v.try_push(file_info).map_err(|_| Error::<T>::StorageLimitReached)?;
				Ok(())
			})?;

			Ok(())
		}

		//Obtain the consensus of the current block
		fn get_current_scheduler() -> AccountOf<T> {
			//Current block information
			let digest = <frame_system::Pallet<T>>::digest();
			let pre_runtime_digests = digest.logs.iter().filter_map(|d| d.as_pre_runtime());
			//TODO
			let acc = T::FindAuthor::find_author(pre_runtime_digests).map(|a| a);
			T::Scheduler::get_controller_acc(acc.unwrap())
		}
	
		fn clear_expired_file(acc: &AccountOf<T>) -> DispatchResult {
			let file_list = <UserHoldFileList<T>>::try_get(&acc).map_err(|_| Error::<T>::Overflow)?;
			for v in file_list.iter() {
				Self::clear_user_file(v.file_hash.clone(), acc)?;
			}

			Ok(())
		}
	}
}

pub trait RandomFileList<AccountId> {
	//Get random challenge data
	fn get_random_challenge_data(
	) -> Result<Vec<(AccountId, Vec<u8>, Vec<u8>, u64, u8, u32)>, DispatchError>;
	//Delete filler file
	fn delete_filler(miner_acc: AccountId, filler_id: Vec<u8>) -> DispatchResult;
	//Delete all filler according to miner_acc
	fn delete_miner_all_filler(miner_acc: AccountId) -> DispatchResult;
	//Delete file backup
	fn clear_file(file_hash: Vec<u8>) -> DispatchResult;
	//The function executed when the challenge fails to let the consensus schedule recover the file
	fn add_invalid_file(miner_acc: AccountId, file_hash: Vec<u8>) -> DispatchResult;
	//Judge whether it is a user who can initiate transactions on the off chain machine
	fn contains_member(acc: AccountId) -> bool;
}

impl<T: Config> RandomFileList<<T as frame_system::Config>::AccountId> for Pallet<T> {
	fn get_random_challenge_data(
	) -> Result<Vec<(AccountOf<T>, Vec<u8>, Vec<u8>, u64, u8, u32)>, DispatchError> {
		let result = Pallet::<T>::get_random_challenge_data()?;
		Ok(result)
	}

	fn delete_filler(miner_acc: AccountOf<T>, filler_id: Vec<u8>) -> DispatchResult {
		Pallet::<T>::delete_filler(miner_acc, filler_id)?;
		Ok(())
	}
	
	fn delete_miner_all_filler(miner_acc: AccountOf<T>) -> DispatchResult {
		let _ = FillerMap::<T>::remove_prefix(&miner_acc, Option::None);
		Ok(())
	}

	fn clear_file(file_hash: Vec<u8>) -> DispatchResult {
		Pallet::<T>::clear_file(file_hash)?;
		Ok(())
	}

	fn add_invalid_file(miner_acc: AccountOf<T>, file_hash: Vec<u8>) -> DispatchResult {
		Pallet::<T>::add_invalid_file(miner_acc, file_hash)?;
		Ok(())
	}

	fn contains_member(acc: AccountOf<T>) -> bool {
		let member_list = Self::members();
		if member_list.contains(&acc) {
			return true
		} else {
			return false
		}
	}
}

impl<T: Config> BlockNumberProvider for Pallet<T> {
	type BlockNumber = T::BlockNumber;

	fn current_block_number() -> Self::BlockNumber {
		<frame_system::Pallet<T>>::block_number()
	}
}
