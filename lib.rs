#![cfg_attr(not(feature = "std"), no_std)]

#[macro_use]
extern crate alloc;

use pink::PinkEnvironment;
use pink_extension as pink;

#[pink::contract(env = PinkEnvironment)]
mod dRuntime {
    use super::*;
    use alloc::string::String;
    use pink::PinkEnvironment;

    use futures::Future;
    use hex_literal::hex;
    use pink::http_get;
    use pink_web3::api::{Eth, Namespace};
    use pink_web3::contract::{Contract, Options};
    use pink_web3::transports::{resolve_ready, PinkHttp};
    use pink_web3::types::TransactionParameters;
    use pink_web3::types::{FilterBuilder, H160};

    #[ink(storage)]
    pub struct Web3 {
        url: String,
    }

    impl Web3 {
        fn eth(&self) -> Eth<PinkHttp> {
            Eth::new(PinkHttp::new(self.url.clone()))
        }

        #[ink(constructor)]
        pub fn default() -> Self {
            Self {
                url: "https://rinkeby.infura.io/v3/".into(),
            }
        }

        fn fetch_contract_events(&self) -> pink_web3::contract::Result<String> {
            // let contract_address = "0xC50fC6Ef39f1436382051562edfe1b70Fb4262b6";
            let contract_address = hex!("d028d24f16a8893bd078259d413372ac01580769").into();
            // let my_account: = hex!("d028d24f16a8893bd078259d413372ac01580769").into();
            // let addr = H160::from_slice(contract_address.as_bytes());
            let contract = Contract::from_json(
                self.eth(),
                contract_address,
                include_bytes!("contract/res/saas3.abi.json"),
            )?;
            // Filter for event in our contract
            let filter = FilterBuilder::default()
                .address(vec![contract_address])
                .topics(None, None, None, None)
                .build();
            pink::debug!("fetching logs");
            let log = self.eth().logs(filter).resolve().map_err(|e| {
                pink::debug!("error: {:?}", e);
                e
            })?;
            for l in log {
                pink::debug!("log: {:?}", l);
            }
            //let filter = self.etheth_filter().create_logs_filter(filter).await?;
            //let logs_stream = filter.stream(time::Duration::from_secs(1));
            //futures::pin_mut!(logs_stream);
            //let log = logs_stream.next().await.unwrap();

            //let result: Future<Output = Result<String>> =
            //    contract.query("getName", (my_account,), None, Options::default(), None);
            //let res: String = result.await?;
            Ok(String::from("Good"))
        }

        //#[pink(on_block_end)]
        #[ink(message)]
        pub fn on_block_end(&self) {
            let block_num = self.env().block_number();
            pink::debug!("block_num: {:?}", block_num);
            // retrieve events every 12*3 seconds
            if block_num % 3 != 0 {
                return;
            }
            self.fetch_contract_events();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        use ink_lang as ink;

        #[ink::test]
        fn tx_works() {
            pink_extension_runtime::mock_ext::mock_all_ext();
            let web3 = Web3::default();
            _ = web3.on_block_end();
            assert_eq!(1, 2);
        }
    }
}
