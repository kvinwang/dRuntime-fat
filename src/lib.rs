#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use ink_lang as ink;

pub use crate::sample_oracle::*;

#[ink::contract(env = pink_extension::PinkEnvironment)]
mod sample_oracle {
    use alloc::{string::String, vec::Vec, string::ToString};
    use ink_storage::traits::{PackedLayout, SpreadLayout};
    use phat_offchain_rollup::{
        clients::evm::read::{Action, QueuedRollupSession},
        lock::GLOBAL as GLOBAL_LOCK,
        RollupHandler, RollupResult,
    };
    use pink_extension as pink;
    use pink_web3::ethabi;
    use primitive_types::H160;
    use scale::{Decode, Encode};

    use abi::ABI;
    use primitive_types::U256;
    use pink::http_get;
    use serde_json;

    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct SampleOracle {
        owner: AccountId,
        config: Option<Config>,
    }

    #[derive(Encode, Decode, Debug, PackedLayout, SpreadLayout)]
    #[cfg_attr(
        feature = "std",
        derive(scale_info::TypeInfo, ink_storage::traits::StorageLayout)
    )]
    struct Config {
        rpc: String,
        anchor: [u8; 20],
    }

    #[derive(Encode, Decode, Debug)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        BadOrigin,
        NotConfigurated,
        BadAbi,
        FailedToGetStorage,
        FailedToDecodeStorage,
        FailedToEstimateGas,
        FailedToDecodeParams,
        FailedToDecodeResBody,
        MultiplyTimesOverflow,
    }

    type Result<T> = core::result::Result<T, Error>;

    impl SampleOracle {
        #[ink(constructor)]
        pub fn default() -> Self {
            Self {
                owner: Self::env().caller(),
                config: None,
            }
        }

        /// Configures the rollup target
        #[ink(message)]
        pub fn config(&mut self, rpc: String, anchor: H160) -> Result<()> {
            self.ensure_owner()?;
            self.config = Some(Config {
                rpc,
                anchor: anchor.into(),
            });
            Ok(())
        }

        fn handle_req(&self) -> Result<Option<RollupResult>> {
            #[cfg(feature = "std")]
            println!("handling req");

            let Config { rpc, anchor } = self.config.as_ref().ok_or(Error::NotConfigurated)?;
            let mut rollup = QueuedRollupSession::new(rpc, anchor.into(), b"q", |_locks| {});

            // Declare write to global lock since it pops an element from the queue
            rollup
                .lock_write(GLOBAL_LOCK)
                .expect("FIXME: failed to fetch lock");

            // Read the first item in the queue (return if the queue is empty)
            let (raw_item, idx) = rollup
                .queue_head()
                .expect("FIXME: failed to read queue head");
            let raw_item = match raw_item {
                Some(v) => v,
                _ => return Ok(None),
            };

            #[cfg(feature = "std")]
            println!("raw_item {:?}", raw_item);

            // Decode the queue data by ethabi (u256, bytes)
            let decoded = ethabi::decode(
                &[ethabi::ParamType::Uint(32), ethabi::ParamType::Bytes],
                &raw_item,
            )
            .or(Err(Error::FailedToDecodeStorage))?;

            let (rid, parameter_abi_bytes) = match decoded.as_slice() {
                [ethabi::Token::Uint(reqid), ethabi::Token::Bytes(content)] => (reqid, content),
                _ => return Err(Error::FailedToDecodeStorage),
            };

            // 
            //let tokens_of_params = ethabi::decode(
            //    &[ethabi::ParamType::Array(Box::new(
            //        ethabi::ParamType::FixedBytes(32),
            //    ))],
            //    &parameter_abi_bytes,
            //);
            //let mut abi256 = Vec::new();
            //for param in tokens_of_params.unwrap().into_iter() {
            //    if let ethabi::Token::FixedBytes(bytes) = param {
            //        //let mut buf = [0u8; 32];
            //        //buf.copy_from_slice(&bytes);
            //        //let v = U256::from_big_endian(&buf);
            //        abi256.push(U256::from_big_endian(&bytes));
            //    }
            //}
            //let decoded_abi = ABI::decode(&abi256, true).or(Err(Error::FailedToDecodeParams))?;
            let decoded_abi = ABI::decode_from_slice(parameter_abi_bytes, true).or(Err(Error::FailedToDecodeParams))?;

            #[cfg(feature = "std")]
            println!("Got decoded params abi {:?}", decoded_abi);

            let url_suffix = decoded_abi
                .params
                .iter()
                .filter(|param| !param.get_name().starts_with("_"))
                .map(|param| param.get_name().to_string() + "=" + &param.get_value().to_string())
                .collect::<Vec<String>>()
                .join("&");

            #[cfg(feature = "std")]
            println!("Got url suffix {:?}", url_suffix);

            let resp = http_get!(
                "https://api.coingecko.com/api/v3/simple/price?".to_string() + &url_suffix
            );

            #[cfg(feature = "std")]
            println!("Got response {:?}", resp.body);

            // TODO check resp code
            let body = resp.body;

            #[derive(serde::Deserialize)]
            struct EthereumUsd {
                ethereum: Usd,
            }

            #[derive(serde::Deserialize)]
            struct Usd {
                usd: String,
            }

            let price: EthereumUsd =
                pink_json::from_slice(&body).or(Err(Error::FailedToDecodeResBody))?;

            let encoded_price = U256::from_dec_str(&price.ethereum.usd).unwrap();
            let (onchain_price, over) = encoded_price.overflowing_mul(1000000.into());
            if over == true {
                return Err(Error::MultiplyTimesOverflow);
            }

            // Apply the response to request
            let payload = ethabi::encode(&[
                ethabi::Token::Uint(*rid),
                ethabi::Token::FixedBytes(onchain_price.encode()),
            ]);

            rollup
                .tx_mut()
                .action(Action::Reply(payload))
                .action(Action::ProcessedTo(idx + 1));

            Ok(Some(rollup.build()))
        }

        /// Returns BadOrigin error if the caller is not the owner
        fn ensure_owner(&self) -> Result<()> {
            if self.env().caller() == self.owner {
                Ok(())
            } else {
                Err(Error::BadOrigin)
            }
        }
    }

    impl RollupHandler for SampleOracle {
        #[ink(message)]
        fn handle_rollup(&self) -> core::result::Result<Option<RollupResult>, Vec<u8>> {
            self.handle_req().map_err(|e| Encode::encode(&e))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use ink_lang as ink;

        fn consts() -> (String, H160) {
            use std::env;
            dotenvy::dotenv().ok();
            /*
             Deployed {
                anchor: '0xb3083F961C729f1007a6A1265Ae6b97dC2Cc16f2',
                oracle: '0x8Bf50F8d0B62017c9B83341CB936797f6B6235dd'
            }
            */
            let rpc = env::var("RPC").unwrap();
            let anchor_addr: [u8; 20] =
                hex::decode(env::var("ANCHOR_ADDR").expect("env not found"))
                    .expect("hex decode failed")
                    .try_into()
                    .expect("invald length");
            let anchor_addr: H160 = anchor_addr.into();
            (rpc, anchor_addr)
        }

        #[ink::test]
        fn default_works() {
            pink_extension_runtime::mock_ext::mock_all_ext();

            let (rpc, anchor_addr) = consts();

            let mut sample_oracle = SampleOracle::default();
            sample_oracle.config(rpc, anchor_addr).unwrap();

            let res = sample_oracle.handle_req().unwrap();
            println!("res: {:#?}", res);
        }

        #[ink::test]
        fn http_get_works() {
            pink_extension_runtime::mock_ext::mock_all_ext();
            let resp = http_get!(
                "https://localhost:3301/saas3/web2/qatar2022/played?home=Qatar&guest=Ecuador"
            );
            assert_eq!(resp.status_code, 200);
            println!("resp: {:#?}", resp.body);


        }
    }
}
