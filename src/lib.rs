#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use ink_lang as ink;
use pink_extension as pink;
//use serde_json;

pub use sample_oracle::*;

#[ink::contract(env = pink_extension::PinkEnvironment)]
mod sample_oracle {
    use super::{pink, serde_json};

    use abi::ABI;
    use alloc::{string::String, vec::Vec};
    use ink_storage::traits::{PackedLayout, SpreadLayout};
    use phat_offchain_rollup::{
        clients::evm::read::{Action, QueuedRollupSession},
        lock::GLOBAL as GLOBAL_LOCK,
        RollupHandler, RollupResult,
    };
    use pink::http_get;
    use primitive_types::U256;
    use pink_web3::ethabi;
    use primitive_types::H160;
    use scale::{Decode, Encode};

    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct SampleOracle {
        owner: AccountId,
        config: Option<Config>,
    }

    #[derive(Encode, Decode, Debug, PackedLayout, SpreadLayout)]
    #[cfg_attr(derive(scale_info::TypeInfo, ink_storage::traits::StorageLayout)
    )]
    struct Config {
        rpc: String,
        anchor: [u8; 20],
        spec: String,
    }

    #[derive(Encode, Decode, Debug)]
    #[cfg_attr(derive(scale_info::TypeInfo))]
    pub enum Error {
        BadOrigin,
        NotConfigurated,
        BadAbi,
        FailedToGetStorage,
        FailedToDecodeStorage,
        FailedToEstimateGas,
        FailedToDecodeParams,
        FailedToDecodeResBody,
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
        pub fn config(&mut self, rpc: String, anchor: H160, spec: String) -> Result<()> {
            self.ensure_owner()?;
            self.config = Some(Config {
                rpc,
                anchor: anchor.into(),
                spec,
            });
            Ok(())
        }

        fn handle_req(&self) -> Result<Option<RollupResult>> {
            let Config { rpc, anchor, spec } =
                self.config.as_ref().ok_or(Error::NotConfigurated)?;
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

            // Decode the queue data by ethabi (u256, bytes)
            let decoded = ethabi::decode(
                &[ethabi::ParamType::Uint(32), ethabi::ParamType::Bytes],
                &raw_item,
            )
            .or(Err(Error::FailedToDecodeStorage))?;

            let (rid, pair) = match decoded.as_slice() {
                [ethabi::Token::Uint(reqid), ethabi::Token::Bytes(content)] => (reqid, content),
                _ => return Err(Error::FailedToDecodeStorage),
            };

            let encoded_params = ethabi::decode(
                &[ethabi::ParamType::Array(Box::new(
                    ethabi::ParamType::FixedBytes(32),
                ))],
                &pair,
            );
            let mut ss = Vec::new();
            for param in encoded_params.unwrap().into_iter() {
                if let ethabi::Token::FixedBytes(bytes) = param {
                    let mut buf = [0u8; 32];
                    buf.copy_from_slice(&bytes);
                    let v = U256::from_big_endian(&buf);
                    ss.push(v);
                }
            }

            // Print the human readable request
            #[cfg(feature = "std")]
            println!("Got req ({}, {:?})", rid, ss);

            let abi = ABI::decode(&ss, true).or(Err(Error::FailedToDecodeParams))?;

            #[cfg(feature = "std")]
            println!("Got decoded params abi {:?}", abi);

            let mut url_params = vec![];
            abi.params.iter().for_each(|param| {
                url_params.push(param.get_name().to_owned() + "=" + &param.get_value().to_owned());
            });
            let url_suffix = abi
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

            // TODO check resp code
            let body = resp.body;
            let root = serde_json::from_slice::<serde_json::Value>(&body)
                .or(Err(Error::FailedToDecodeResBody))?;

            // TODO use macro to generate the code
            // 1. get path field
            // 2. generate the code
            let price = root.get("ethereum").and_then(|value| value.get("usd")).and_then(|value|value.as_str()).unwrap();

            
            let encoded_price = U256::from_dec_str(price).unwrap();

            //let mut price_bytes = [0u8; 32];
            //encoded_price.to_big_endian(&mut price_bytes);
            // Apply the response to request
            //let payload = ethabi::encode(&[
            //    ethabi::Token::Uint(*rid),
            //    ethabi::Token::FixedBytes(price_bytes.to_vec()),
            //]);
            let payload = ethabi::encode(&[
                ethabi::Token::Uint(*rid),
                ethabi::Token::Uint(encoded_price),
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
    }
}
