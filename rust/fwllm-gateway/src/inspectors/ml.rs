use std::path::Path;
use std::collections::HashMap;

pub trait TextClassifier: Send + Sync {
    fn predict(&self, text: &str) -> (String, f32);
}

pub struct OnnxClassifier {
    session: parking_lot::Mutex<ort::session::Session>,
    tokenizer: parking_lot::Mutex<tokenizers::Tokenizer>,
    id2label: HashMap<i64, String>,
}

impl OnnxClassifier {
    pub fn from_dir(dir: &Path) -> anyhow::Result<Self> {
        let model_path = dir.join("model.onnx");
        let tokenizer_path = dir.join("tokenizer.json");
        let config_path = dir.join("config.json");
        let session = ort::session::Session::builder()?
            .commit_from_file(&model_path)?;
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("tokenizer load: {e}"))?;
        let id2label = if config_path.exists() {
            let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&config_path)?)?;
            cfg.get("id2label").and_then(|v| v.as_object()).map(|m| {
                m.iter().filter_map(|(k,v)| Some((k.parse::<i64>().ok()?, v.as_str()?.to_string()))).collect()
            }).unwrap_or_else(|| [(0, "benign".into()), (1, "injection".into())].into())
        } else {
            [(0, "benign".into()), (1, "injection".into())].into()
        };
        Ok(Self { session: parking_lot::Mutex::new(session), tokenizer: parking_lot::Mutex::new(tokenizer), id2label })
    }
}

impl TextClassifier for OnnxClassifier {
    fn predict(&self, text: &str) -> (String, f32) {
        // Truncate to max_length and handle errors without panicking
        let truncated = &text[..text.len().min(4096)];
        let encoding = match self.tokenizer.lock().encode(truncated, true) {
            Ok(enc) => enc,
            Err(_) => return ("benign".to_string(), 0.0),
        };
        let mut ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
        if ids.is_empty() {
            return ("benign".to_string(), 0.0);
        }
        // Truncate to model's max length (512 for BERT-like)
        const MAX_LEN: usize = 512;
        if ids.len() > MAX_LEN {
            ids.truncate(MAX_LEN);
        }
        let mask: Vec<i64> = vec![1; ids.len()];
        let ids_array = match ndarray::Array2::from_shape_vec((1, ids.len()), ids) {
            Ok(a) => a,
            Err(_) => return ("benign".to_string(), 0.0),
        };
        let mask_array = match ndarray::Array2::from_shape_vec((1, mask.len()), mask) {
            Ok(a) => a,
            Err(_) => return ("benign".to_string(), 0.0),
        };
        let ids_value = match ort::value::Value::from_array(ids_array) {
            Ok(v) => v,
            Err(_) => return ("benign".to_string(), 0.0),
        };
        let mask_value = match ort::value::Value::from_array(mask_array) {
            Ok(v) => v,
            Err(_) => return ("benign".to_string(), 0.0),
        };
        let mut session = self.session.lock();
        let outputs = match session.run(ort::inputs!["input_ids" => ids_value.view(), "attention_mask" => mask_value.view()]) {
            Ok(o) => o,
            Err(_) => return ("benign".to_string(), 0.0),
        };
        let (_shape, data) = match outputs[0].try_extract_tensor::<f32>() {
            Ok(t) => t,
            Err(_) => return ("benign".to_string(), 0.0),
        };
        let logits: Vec<f32> = data.to_vec();
        let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let exps: Vec<f32> = logits.iter().map(|x| (x - max).exp()).collect();
        let sum: f32 = exps.iter().sum();
        if sum == 0.0 {
            return ("benign".to_string(), 0.0);
        }
        let probs: Vec<f32> = exps.iter().map(|x| x / sum).collect();
        let best = probs.iter().enumerate().max_by(|a,b| a.1.partial_cmp(b.1).unwrap()).map(|(i,_)| i).unwrap_or(0) as i64;
        let label = self.id2label.get(&best).cloned().unwrap_or_else(|| best.to_string());
        (label, probs[best as usize])
    }
}

pub fn try_load_classifier(dir: &str) -> Option<Box<dyn TextClassifier>> {
    let path = Path::new(dir);
    if !path.join("model.onnx").exists() { return None; }
    match OnnxClassifier::from_dir(path) {
        Ok(c) => Some(Box::new(c)),
        Err(e) => {
            tracing::warn!("ml classifier load failed: {e}");
            None
        }
    }
}

pub struct MlInjectionInspector {
    classifier: Box<dyn TextClassifier>,
    threshold: f32,
    block_gte: String,
    mode: String,
}

impl MlInjectionInspector {
    pub fn new(classifier: Box<dyn TextClassifier>, threshold: f32, block_gte: String, mode: String) -> Self {
        Self { classifier, threshold, block_gte, mode }
    }

    pub fn scan(&self, text: &str) -> Option<(&'static str, &'static str)> {
        let (label, conf) = self.classifier.predict(text);
        if label == "benign" || conf < self.threshold { return None; }
        let severity: &'static str = if conf >= 0.9 { "critical" } else if conf >= 0.8 { "high" } else if conf >= 0.7 { "medium" } else { "low" };
        Some(("ml:injection", severity))
    }
}
