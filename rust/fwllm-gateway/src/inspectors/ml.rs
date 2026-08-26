use std::path::Path;
use std::collections::HashMap;

pub trait TextClassifier: Send + Sync {
    fn predict(&self, text: &str) -> (String, f32);
}

pub struct OnnxClassifier {
    session: std::sync::Mutex<ort::session::Session>,
    tokenizer: std::sync::Mutex<tokenizers::Tokenizer>,
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
        Ok(Self { session: std::sync::Mutex::new(session), tokenizer: std::sync::Mutex::new(tokenizer), id2label })
    }
}

impl TextClassifier for OnnxClassifier {
    fn predict(&self, text: &str) -> (String, f32) {
        let encoding = self.tokenizer.lock().unwrap().encode(text, true).unwrap();
        let ids: Vec<i64> = encoding.get_ids().iter().map(|&x| x as i64).collect();
        let mask: Vec<i64> = vec![1; ids.len()];
        let ids_array = ndarray::Array2::from_shape_vec((1, ids.len()), ids).unwrap();
        let mask_array = ndarray::Array2::from_shape_vec((1, mask.len()), mask).unwrap();
        let ids_value = ort::value::Value::from_array(ids_array).unwrap();
        let mask_value = ort::value::Value::from_array(mask_array).unwrap();
        let mut session = self.session.lock().unwrap();
        let outputs = session.run(ort::inputs!["input_ids" => ids_value.view(), "attention_mask" => mask_value.view()]).unwrap();
        let (_shape, data) = outputs[0].try_extract_tensor::<f32>().unwrap();
        let logits: Vec<f32> = data.to_vec();
        let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let exps: Vec<f32> = logits.iter().map(|x| (x - max).exp()).collect();
        let sum: f32 = exps.iter().sum();
        let probs: Vec<f32> = exps.iter().map(|x| x / sum).collect();
        let best = probs.iter().enumerate().max_by(|a,b| a.1.partial_cmp(b.1).unwrap()).unwrap().0 as i64;
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
