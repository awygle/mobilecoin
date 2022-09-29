// Copyright (c) 2018-2022 The MobileCoin Foundation

//! Contains methods that allow a Fog View Router enclave to combine all of the
//! Fog View Shard's query responses into one query response that'll be returned
//! for the client.

use aligned_cmov::{
    subtle::{Choice, ConstantTimeEq},
    CMov,
};
use alloc::vec::Vec;
use mc_fog_types::ledger::{KeyImageQuery, KeyImageResult, KeyImageResultCode};
use mc_watcher_api::TimestampResultCode;

/// The default KeyImageResultCode used when collating the shard responses.
const DEFAULT_KEY_IMAGE_SEARCH_RESULT_CODE: KeyImageResultCode = KeyImageResultCode::NotSpent;

pub fn collate_shard_key_image_search_results(
    client_queries: Vec<KeyImageQuery>,
    shard_key_image_search_results: Vec<KeyImageResult>,
) -> Vec<KeyImageResult> {
    let mut client_key_image_search_results: Vec<KeyImageResult> = client_queries
        .iter()
        .map(|client_query| KeyImageResult {
            key_image: client_query.key_image.clone(),
            spent_at: 1, // not 0 because it's defined to be >0 in the .proto file
            timestamp: u64::MAX,
            timestamp_result_code: TimestampResultCode::TimestampFound as u32,
            key_image_result_code: DEFAULT_KEY_IMAGE_SEARCH_RESULT_CODE as u32,
        })
        .collect();

    for shard_key_image_search_result in shard_key_image_search_results.iter() {
        for client_key_image_search_result in client_key_image_search_results.iter_mut() {
            maybe_overwrite_key_image_search_result(
                client_key_image_search_result,
                shard_key_image_search_result,
            );
        }
    }

    client_key_image_search_results
}

fn maybe_overwrite_key_image_search_result(
    client_key_image_search_result: &mut KeyImageResult,
    shard_key_image_search_result: &KeyImageResult,
) {
    let should_overwrite_key_image_search_result = should_overwrite_key_image_search_result(
        client_key_image_search_result,
        shard_key_image_search_result,
    );

    client_key_image_search_result.key_image_result_code.cmov(
        should_overwrite_key_image_search_result,
        &shard_key_image_search_result.key_image_result_code,
    );

    client_key_image_search_result.spent_at.cmov(
        should_overwrite_key_image_search_result,
        &shard_key_image_search_result.spent_at,
    );

    client_key_image_search_result.timestamp.cmov(
        should_overwrite_key_image_search_result,
        &shard_key_image_search_result.timestamp,
    );

    client_key_image_search_result.timestamp_result_code.cmov(
        should_overwrite_key_image_search_result,
        &shard_key_image_search_result.timestamp_result_code,
    );
}

fn should_overwrite_key_image_search_result(
    client_key_image_search_result: &KeyImageResult,
    shard_key_image_search_result: &KeyImageResult,
) -> Choice {
    let client_key_image: &[u8] = client_key_image_search_result.key_image.as_ref();
    let shard_key_image: &[u8] = shard_key_image_search_result.key_image.as_ref();
    let do_key_images_match = client_key_image.ct_eq(&shard_key_image);

    let client_key_image_search_result_code = client_key_image_search_result.key_image_result_code;
    let shard_key_image_search_result_code = shard_key_image_search_result.key_image_result_code;

    let client_code_is_not_spent: Choice =
        client_key_image_search_result_code.ct_eq(&(KeyImageResultCode::NotSpent as u32));

    let shard_code_is_spent: Choice =
        shard_key_image_search_result_code.ct_eq(&(KeyImageResultCode::Spent as u32));
    let shard_code_is_error: Choice =
        shard_key_image_search_result_code.ct_eq(&(KeyImageResultCode::KeyImageError as u32));

    //   We make the same query to several shards and get several responses, and
    // this logic determines how we fill the one client response.
    //   At a high level, we want to prioritize "spent" responses.
    // Error responses are "retriable" errors that the client will retry
    // after a backoff. The "not spent" response is the default response and
    // gets overwritten by any other response.
    // spent > error > not spent
    do_key_images_match
           // Always write a Found code
        & (shard_code_is_spent
            // Write an error code IFF the client code is NotFound.
            | ((shard_code_is_error) & client_code_is_not_spent))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use itertools::Itertools;
    use std::collections::HashSet;

    #[test]
    fn should_overwrite_tests() {
        // Images don't match
        let client_result = KeyImageResult {
            key_image: 123456u64.into(),
            spent_at: 1,
            timestamp: 123456,
            timestamp_result_code: TimestampResultCode::TimestampFound as u32,
            key_image_result_code: DEFAULT_KEY_IMAGE_SEARCH_RESULT_CODE as u32,
        };
        let shard_result = KeyImageResult {
            key_image: 654321u64.into(),
            spent_at: 1,
            timestamp: 123456,
            timestamp_result_code: TimestampResultCode::TimestampFound as u32,
            key_image_result_code: KeyImageResultCode::Spent as u32,
        };
        assert!(!should_overwrite_key_image_search_result(
            client_result,
            shard_result
        ));
    }
    /*
    #[test]
    fn collate_shard_query_responses_shards_find_all_tx_outs() {
        let client_search_keys: Vec<Vec<u8>> = (0..10).map(|num| vec![num; 10]).collect();
        let shard_tx_out_search_results: Vec<TxOutSearchResult> = client_search_keys
            .iter()
            .map(|search_key| {
                create_test_tx_out_search_result(
                    search_key.clone(),
                    0,
                    CLIENT_CIPHERTEXT_LENGTH - 1,
                    TxOutSearchResultCode::Found,
                )
            })
            .collect();

        let result = collate_shard_tx_out_search_results(
            client_search_keys.clone(),
            shard_tx_out_search_results,
        )
        .unwrap();

        let all_tx_out_found = result.iter().all(|tx_out_search_result| {
            tx_out_search_result.result_code == TxOutSearchResultCode::Found as u32
        });
        assert!(all_tx_out_found);

        let result_client_search_keys: HashSet<Vec<u8>> = HashSet::from_iter(
            result
                .iter()
                .map(|tx_out_search_result| tx_out_search_result.search_key.clone()),
        );
        assert_eq!(
            result_client_search_keys,
            HashSet::from_iter(client_search_keys)
        );
    }

    #[test]
    fn collate_shard_query_responses_shards_one_not_found() {
        let client_search_keys: Vec<Vec<u8>> = (0..10).map(|num| vec![num; 10]).collect();
        let shard_tx_out_search_results: Vec<TxOutSearchResult> = client_search_keys
            .iter()
            .enumerate()
            .map(|(i, search_key)| {
                let result_code = match i {
                    0 => TxOutSearchResultCode::NotFound,
                    _ => TxOutSearchResultCode::Found,
                };
                create_test_tx_out_search_result(
                    search_key.clone(),
                    0,
                    CLIENT_CIPHERTEXT_LENGTH - 1,
                    result_code,
                )
            })
            .collect();

        let result = collate_shard_tx_out_search_results(
            client_search_keys.clone(),
            shard_tx_out_search_results,
        )
        .unwrap();

        let result_client_search_keys: HashSet<Vec<u8>> = HashSet::from_iter(
            result
                .iter()
                .map(|tx_out_search_result| tx_out_search_result.search_key.clone()),
        );
        assert_eq!(
            result_client_search_keys,
            HashSet::from_iter(client_search_keys)
        );

        let not_found_count = result
            .iter()
            .filter(|tx_out_search_result| {
                tx_out_search_result.result_code == TxOutSearchResultCode::NotFound as u32
            })
            .count();
        assert_eq!(not_found_count, 1);
    }

    #[test]
    fn collate_shard_query_responses_ciphertext_is_client_ciphertext_length_panics() {
        let client_search_keys: Vec<Vec<u8>> = (0..10).map(|num| vec![num; 10]).collect();
        let shard_tx_out_search_results: Vec<TxOutSearchResult> = client_search_keys
            .iter()
            .map(|search_key| TxOutSearchResult {
                search_key: search_key.clone(),
                result_code: TxOutSearchResultCode::NotFound as u32,
                ciphertext: vec![0u8; CLIENT_CIPHERTEXT_LENGTH],
            })
            .collect();

        let result = std::panic::catch_unwind(|| {
            collate_shard_tx_out_search_results(
                client_search_keys.clone(),
                shard_tx_out_search_results,
            )
        });

        assert!(result.is_err());
    }
    #[test]
    fn collate_shard_query_responses_different_ciphertext_lengths_returns_correct_client_ciphertexts(
    ) {
        let client_search_keys: Vec<Vec<u8>> = (0..3).map(|num| vec![num; 10]).collect();
        let ciphertext_values = [28u8, 5u8, 128u8];
        let shard_tx_out_search_results: Vec<TxOutSearchResult> = client_search_keys
            .iter()
            .enumerate()
            .map(|(idx, search_key)| TxOutSearchResult {
                search_key: search_key.clone(),
                result_code: TxOutSearchResultCode::Found as u32,
                ciphertext: vec![ciphertext_values[idx]; idx + 1],
            })
            .collect();

        let results: Vec<TxOutSearchResult> =
            collate_shard_tx_out_search_results(client_search_keys, shard_tx_out_search_results)
                .unwrap()
                .into_iter()
                // Sort by ciphertext length (ascending) in order to know what each expected result
                // should be.
                .sorted_by(|a, b| Ord::cmp(&b.ciphertext[0], &a.ciphertext[0]))
                .collect();

        let mut expected_first_result = [0u8; CLIENT_CIPHERTEXT_LENGTH];
        let expected_first_result_delta = (CLIENT_CIPHERTEXT_LENGTH - 1) as u8;
        expected_first_result[0] = expected_first_result_delta;
        expected_first_result[1] = ciphertext_values[0];
        assert_eq!(results[0].ciphertext, expected_first_result);

        let mut expected_second_result = [0u8; CLIENT_CIPHERTEXT_LENGTH];
        let expected_second_result_delta = (CLIENT_CIPHERTEXT_LENGTH - 2) as u8;
        expected_second_result[0] = expected_second_result_delta;
        expected_second_result[1] = ciphertext_values[1];
        expected_second_result[2] = ciphertext_values[1];
        assert_eq!(results[1].ciphertext, expected_second_result);

        let mut expected_third_result = [0u8; CLIENT_CIPHERTEXT_LENGTH];
        let expected_third_result_delta = (CLIENT_CIPHERTEXT_LENGTH - 3) as u8;
        expected_third_result[0] = expected_third_result_delta;
        expected_third_result[1] = ciphertext_values[2];
        expected_third_result[2] = ciphertext_values[2];
        expected_third_result[3] = ciphertext_values[2];
        assert_eq!(results[2].ciphertext, expected_third_result);
    }*/
}
