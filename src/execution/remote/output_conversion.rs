//! Convert nbformat outputs to Y.js CRDT structures

use nbformat::v4::Output;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use yrs::{Any, Array, ArrayPrelim, ArrayRef, Map, MapPrelim, TransactionMut};

/// Convert an nbformat Output to a MapPrelim that can be inserted into the outputs array
#[allow(dead_code)]
pub fn output_to_map_prelim(output: &Output) -> MapPrelim {
    match output {
        Output::Stream { name, text } => MapPrelim::from([
            ("output_type", "stream"),
            ("name", name.as_str()),
            ("text", text.0.as_str()),
        ]),
        Output::DisplayData(display) => {
            // Serialize Media to JSON
            let data_json = serde_json::to_value(&display.data).unwrap_or(JsonValue::Null);
            let metadata_json = serde_json::to_value(&display.metadata)
                .unwrap_or(JsonValue::Object(Default::default()));

            let mut map_entries = vec![("output_type", Any::String("display_data".into()))];
            map_entries.push(("data", json_to_any(&data_json)));
            map_entries.push(("metadata", json_to_any(&metadata_json)));

            map_entries.into_iter().collect()
        }
        Output::ExecuteResult(result) => {
            // Serialize Media to JSON
            let data_json = serde_json::to_value(&result.data).unwrap_or(JsonValue::Null);
            let metadata_json = serde_json::to_value(&result.metadata)
                .unwrap_or(JsonValue::Object(Default::default()));

            let mut map_entries = vec![
                ("output_type", Any::String("execute_result".into())),
                (
                    "execution_count",
                    Any::BigInt(result.execution_count.0 as i64),
                ),
            ];
            map_entries.push(("data", json_to_any(&data_json)));
            map_entries.push(("metadata", json_to_any(&metadata_json)));

            map_entries.into_iter().collect()
        }
        Output::Error(error) => {
            // Convert traceback to Vec<Any>
            let traceback_vec: Vec<Any> = error
                .traceback
                .iter()
                .map(|line| Any::String(line.clone().into()))
                .collect();

            MapPrelim::from([
                ("output_type", Any::String("error".into())),
                ("ename", Any::String(error.ename.clone().into())),
                ("evalue", Any::String(error.evalue.clone().into())),
                ("traceback", Any::from(traceback_vec)),
            ])
        }
    }
}

/// Convert a serde_json::Value to a yrs::Any that can be inserted into Y.js structures
fn json_to_any(value: &JsonValue) -> Any {
    match value {
        JsonValue::Null => Any::Null,
        JsonValue::Bool(b) => Any::Bool(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Any::BigInt(i)
            } else if let Some(f) = n.as_f64() {
                Any::Number(f)
            } else {
                Any::Null
            }
        }
        JsonValue::String(s) => Any::String(s.clone().into()),
        JsonValue::Array(arr) => {
            let vec: Vec<Any> = arr.iter().map(json_to_any).collect();
            Any::from(vec)
        }
        JsonValue::Object(obj) => {
            let map: HashMap<String, Any> = obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_any(v)))
                .collect();
            Any::from(map)
        }
    }
}

/// Update a cell's outputs in the Y.js document
#[allow(dead_code)]
pub fn update_cell_outputs(
    txn: &mut TransactionMut,
    cells_array: &ArrayRef,
    cell_index: usize,
    outputs: &[Output],
) -> Result<(), anyhow::Error> {
    // Get the cell value at the given index
    let cell_value = cells_array
        .get(txn, cell_index as u32)
        .ok_or_else(|| anyhow::anyhow!("Cell index {} out of bounds", cell_index))?;

    // Cast to MapRef
    let cell_map = cell_value
        .cast::<yrs::MapRef>()
        .map_err(|_| anyhow::anyhow!("Cell at index {} is not a Map", cell_index))?;

    // Get or create the outputs array
    let outputs_array: ArrayRef = if let Some(outputs_val) = cell_map.get(txn, "outputs") {
        let arr_ref: ArrayRef = outputs_val
            .cast::<ArrayRef>()
            .map_err(|_| anyhow::anyhow!("Outputs field is not an Array"))?;
        arr_ref
    } else {
        // Create new array if it doesn't exist
        cell_map.insert(txn, "outputs", ArrayPrelim::default())
    };

    // Clear existing outputs
    let current_len = outputs_array.len(txn);
    if current_len > 0 {
        outputs_array.remove_range(txn, 0, current_len);
    }

    // Add new outputs
    for (i, output) in outputs.iter().enumerate() {
        let output_map = output_to_map_prelim(output);
        outputs_array.insert(txn, i as u32, output_map);
    }

    Ok(())
}

/// Update a cell's execution_count in the Y.js document
#[allow(dead_code)]
pub fn update_cell_execution_count(
    txn: &mut TransactionMut,
    cells_array: &ArrayRef,
    cell_index: usize,
    execution_count: Option<i64>,
) -> Result<(), anyhow::Error> {
    // Get the cell value at the given index
    let cell_value = cells_array
        .get(txn, cell_index as u32)
        .ok_or_else(|| anyhow::anyhow!("Cell index {} out of bounds", cell_index))?;

    // Cast to MapRef - cast() returns Result, not Option
    let cell_map = cell_value
        .cast::<yrs::MapRef>()
        .map_err(|_| anyhow::anyhow!("Cell at index {} is not a Map", cell_index))?;

    // Update execution_count
    if let Some(count) = execution_count {
        cell_map.insert(txn, "execution_count", count);
    } else {
        cell_map.insert(txn, "execution_count", Any::Null);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nbformat::v4::Output;
    use serde_json::json;
    use yrs::{Doc, Transact};

    fn setup_doc() -> (Doc, yrs::ArrayRef) {
        let doc = Doc::new();
        let cells = doc.get_or_insert_array("cells");
        let mut txn = doc.transact_mut();
        cells.insert(&mut txn, 0, MapPrelim::from([("cell_type", "code")]));
        drop(txn);
        (doc, cells)
    }

    fn stream_out(name: &str, text: &str) -> Output {
        Output::Stream {
            name: name.to_string(),
            text: nbformat::v4::MultilineString(text.to_string()),
        }
    }

    fn execute_result_out(ec: i64) -> Output {
        serde_json::from_value(json!({
            "output_type": "execute_result",
            "execution_count": ec,
            "data": {"text/plain": "val"},
            "metadata": {}
        }))
        .unwrap()
    }

    fn error_out(ename: &str, traceback: &[&str]) -> Output {
        serde_json::from_value(json!({
            "output_type": "error",
            "ename": ename,
            "evalue": "err",
            "traceback": traceback
        }))
        .unwrap()
    }

    fn display_data_out() -> Output {
        serde_json::from_value(json!({
            "output_type": "display_data",
            "data": {"text/plain": "val"},
            "metadata": {}
        }))
        .unwrap()
    }

    fn get_output_map(
        outputs_arr: &yrs::ArrayRef,
        txn: &impl yrs::ReadTxn,
        idx: u32,
    ) -> yrs::MapRef {
        outputs_arr
            .get(txn, idx)
            .unwrap()
            .cast::<yrs::MapRef>()
            .unwrap()
    }

    #[test]
    fn test_all_output_types_stored_with_correct_output_type_field() {
        let (doc, cells) = setup_doc();
        let outputs = vec![
            stream_out("stdout", "hi"),
            execute_result_out(1),
            error_out("NameError", &[]),
            display_data_out(),
        ];
        {
            let mut txn = doc.transact_mut();
            update_cell_outputs(&mut txn, &cells, 0, &outputs).unwrap();
        }
        let txn = doc.transact();
        let cell_map = cells.get(&txn, 0).unwrap().cast::<yrs::MapRef>().unwrap();
        let outputs_arr = cell_map
            .get(&txn, "outputs")
            .unwrap()
            .cast::<yrs::ArrayRef>()
            .unwrap();

        let expected = ["stream", "execute_result", "error", "display_data"];
        for (i, expected_type) in expected.iter().enumerate() {
            let out_map = get_output_map(&outputs_arr, &txn, i as u32);
            let ot = out_map.get(&txn, "output_type").unwrap();
            assert_eq!(
                ot,
                yrs::Out::Any(Any::String((*expected_type).into())),
                "output[{}] output_type wrong",
                i
            );
        }
    }

    #[test]
    fn test_execute_result_stores_execution_count_as_bigint_not_float() {
        let (doc, cells) = setup_doc();
        let outputs = vec![execute_result_out(42)];
        {
            let mut txn = doc.transact_mut();
            update_cell_outputs(&mut txn, &cells, 0, &outputs).unwrap();
        }
        let txn = doc.transact();
        let cell_map = cells.get(&txn, 0).unwrap().cast::<yrs::MapRef>().unwrap();
        let outputs_arr = cell_map
            .get(&txn, "outputs")
            .unwrap()
            .cast::<yrs::ArrayRef>()
            .unwrap();
        let out_map = get_output_map(&outputs_arr, &txn, 0);
        match out_map.get(&txn, "execution_count").unwrap() {
            yrs::Out::Any(Any::BigInt(n)) => assert_eq!(n, 42),
            other => panic!("execution_count must be BigInt(42), got {:?}", other),
        }
    }

    #[test]
    fn test_error_traceback_stored_as_array() {
        let (doc, cells) = setup_doc();
        let outputs = vec![error_out("NameError", &["line1", "line2"])];
        {
            let mut txn = doc.transact_mut();
            update_cell_outputs(&mut txn, &cells, 0, &outputs).unwrap();
        }
        let txn = doc.transact();
        let cell_map = cells.get(&txn, 0).unwrap().cast::<yrs::MapRef>().unwrap();
        let outputs_arr = cell_map
            .get(&txn, "outputs")
            .unwrap()
            .cast::<yrs::ArrayRef>()
            .unwrap();
        let out_map = get_output_map(&outputs_arr, &txn, 0);
        match out_map.get(&txn, "traceback").unwrap() {
            yrs::Out::Any(Any::Array(arr)) => assert_eq!(arr.len(), 2),
            other => panic!("traceback must be Any::Array of 2, got {:?}", other),
        }
    }

    #[test]
    fn test_update_replaces_existing_outputs() {
        let (doc, cells) = setup_doc();
        {
            let mut txn = doc.transact_mut();
            update_cell_outputs(
                &mut txn,
                &cells,
                0,
                &[stream_out("stdout", "a"), stream_out("stderr", "b")],
            )
            .unwrap();
        }
        {
            let mut txn = doc.transact_mut();
            update_cell_outputs(&mut txn, &cells, 0, &[stream_out("stdout", "only")]).unwrap();
        }
        let txn = doc.transact();
        let cell_map = cells.get(&txn, 0).unwrap().cast::<yrs::MapRef>().unwrap();
        let outputs_arr = cell_map
            .get(&txn, "outputs")
            .unwrap()
            .cast::<yrs::ArrayRef>()
            .unwrap();
        assert_eq!(
            outputs_arr.len(&txn),
            1,
            "second update must replace first — only 1 output expected"
        );
        let out_map = get_output_map(&outputs_arr, &txn, 0);
        assert_eq!(
            out_map.get(&txn, "text"),
            Some(yrs::Out::Any(Any::String("only".into())))
        );
    }

    #[test]
    fn test_update_out_of_bounds_returns_err_not_panic() {
        let (doc, cells) = setup_doc();
        let mut txn = doc.transact_mut();
        let result = update_cell_outputs(&mut txn, &cells, 99, &[]);
        assert!(
            result.is_err(),
            "cell_index=99 on 1-cell doc must return Err"
        );
    }

    #[test]
    fn test_stream_output_name_and_text_stored() {
        let (doc, cells) = setup_doc();
        {
            let mut txn = doc.transact_mut();
            update_cell_outputs(&mut txn, &cells, 0, &[stream_out("stdout", "hello")]).unwrap();
        }
        let txn = doc.transact();
        let cell_map = cells.get(&txn, 0).unwrap().cast::<yrs::MapRef>().unwrap();
        let outputs_arr = cell_map
            .get(&txn, "outputs")
            .unwrap()
            .cast::<yrs::ArrayRef>()
            .unwrap();
        let out_map = get_output_map(&outputs_arr, &txn, 0);
        assert_eq!(
            out_map.get(&txn, "name"),
            Some(yrs::Out::Any(Any::String("stdout".into()))),
            "stream name must be 'stdout'"
        );
        assert_eq!(
            out_map.get(&txn, "text"),
            Some(yrs::Out::Any(Any::String("hello".into()))),
            "stream text must be 'hello'"
        );
    }

    #[test]
    fn test_update_execution_count_null() {
        let (doc, cells) = setup_doc();
        {
            let mut txn = doc.transact_mut();
            update_cell_execution_count(&mut txn, &cells, 0, None).unwrap();
        }
        let txn = doc.transact();
        let cell_map = cells.get(&txn, 0).unwrap().cast::<yrs::MapRef>().unwrap();
        assert_eq!(
            cell_map.get(&txn, "execution_count"),
            Some(yrs::Out::Any(Any::Null)),
            "execution_count=None must store Null"
        );
    }

    #[test]
    fn test_update_execution_count_positive() {
        let (doc, cells) = setup_doc();
        {
            let mut txn = doc.transact_mut();
            update_cell_execution_count(&mut txn, &cells, 0, Some(7)).unwrap();
        }
        let txn = doc.transact();
        let cell_map = cells.get(&txn, 0).unwrap().cast::<yrs::MapRef>().unwrap();
        match cell_map.get(&txn, "execution_count").unwrap() {
            yrs::Out::Any(Any::BigInt(n)) => assert_eq!(n, 7),
            yrs::Out::Any(Any::Number(n)) => assert_eq!(n as i64, 7),
            other => panic!("expected BigInt(7) or Number(7), got {:?}", other),
        }
    }
}
