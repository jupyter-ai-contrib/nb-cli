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
