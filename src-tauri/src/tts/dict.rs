//! 纠音词典：在文本送入 TTS 引擎前，将易错读音替换为同音字。
//!
//! G2P (Grapheme-to-Phoneme) 模块常常无法正确处理所有多音字或专有名词。
//! 通过最原始的字符串替换，把多音字替换为发音准确的“白字”，
//! 是在不微调底层模型情况下最轻量的纠音手段。

pub struct PronunciationDict;

impl PronunciationDict {
    /// 预处理 TTS 文本，纠正发音错误
    pub fn preprocess(text: &str) -> String {
        let mut processed = text.to_string();

        // [替换规则]
        // 格式：(原词, 替换词)
        // 注意：替换词只需要“读音一致”，大模型不会把替换后的文本显示给用户，
        // 这仅仅是发给音频合成引擎的内部文本。
        let rules = [
            ("凡人修仙传", "凡人修仙赚"),
            ("传记", "赚记"),
            ("自传", "自赚"),
            ("水浒传", "水浒赚"),
            ("银行", "银航"),
            ("AI", "诶爱"),
            ("ASTS", "A S T S"),
            // 在这里添加更多易错词...
        ];

        for (from, to) in rules.iter() {
            processed = processed.replace(from, to);
        }

        processed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preprocess() {
        let original = "我很喜欢看《凡人修仙传》这部传记！";
        let processed = PronunciationDict::preprocess(original);
        assert_eq!(processed, "我很喜欢看《凡人修仙赚》这部赚记！");
    }
}
