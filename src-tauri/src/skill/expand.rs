//! 把技能正文里的参数占位符替换成实际参数。
//!
//! 约定照 Claude Code：`$ARGUMENTS` 是全部参数原文，`$1`..`$9` 是按空白
//! 切分后的第 N 个。刻意只支持这两种 —— 引一个模板引擎进来，代价是
//! 技能作者要学一门新语法，而收益只是几个用不到的功能。
//!
//! ## 为什么必须一次扫描而不是链式 replace
//!
//! ```ignore
//! body.replace("$ARGUMENTS", args).replace("$1", arg1)   // 错
//! ```
//!
//! 参数本身含 `$1` 时（路径 `src/$1.ts`、正则 `s/(\w+)/$1/`）会被第二次
//! replace 再替换一遍。而这种输入不罕见：让技能去处理一段正则替换就会出现。
//! 一次扫描、遇到占位符就写入参数、写完继续往后走，替换进去的内容不再参与匹配。

/// 替换正文里的参数占位符。
///
/// `args` 是 `/` 后面或工具参数里给的原始字符串，不做任何解析地作为
/// `$ARGUMENTS`；按空白切分后填 `$1`..`$9`。
///
/// 越界的编号（`$7` 但只给了两个参数）替换成空串而不是保留原文 ——
/// 保留 `$7` 的话模型会把它当成字面内容读，然后在回复里问「$7 是什么」。
pub fn substitute(body: &str, args: &str) -> String {
    let args = args.trim();
    let positional: Vec<&str> = args.split_whitespace().collect();

    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len() + args.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            // 按字节推进但只在 ASCII `$` 处切分 —— 多字节字符的任何字节都
            // 不等于 0x24，所以不会切在字符中间
            let start = i;
            while i < bytes.len() && bytes[i] != b'$' {
                i += 1;
            }
            out.push_str(&body[start..i]);
            continue;
        }
        // 这里 bytes[i] == b'$'
        let rest = &body[i..];
        if let Some(after) = rest.strip_prefix("$ARGUMENTS") {
            out.push_str(args);
            i = body.len() - after.len();
            continue;
        }
        // `$1`..`$9`
        let digit = rest.as_bytes().get(1).copied().filter(|c| c.is_ascii_digit());
        match digit {
            Some(d) if d != b'0' => {
                let idx = (d - b'1') as usize;
                if let Some(v) = positional.get(idx) {
                    out.push_str(v);
                }
                i += 2;
            }
            // `$0`、`$x`、行尾的孤立 `$`：原样保留。
            // shell 变量、jQuery、正则里的 `$` 都很常见，不该被吃掉
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
    out
}

/// 正文里到底用了哪些占位符。给界面提示「这个命令需要参数」用。
pub fn placeholders(body: &str) -> Placeholders {
    Placeholders {
        arguments: body.contains("$ARGUMENTS"),
        max_positional: (1..=9u8)
            .filter(|n| body.contains(&format!("${n}")))
            .max()
            .unwrap_or(0),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Placeholders {
    pub arguments: bool,
    /// 用到的最大位置参数编号，0 表示没用位置参数
    pub max_positional: u8,
}

impl Placeholders {
    pub fn takes_args(&self) -> bool {
        self.arguments || self.max_positional > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_是原文而位置参数按空白切() {
        assert_eq!(
            substitute("审查 $ARGUMENTS，重点看 $1", "src/lru.ts src/persist.ts"),
            "审查 src/lru.ts src/persist.ts，重点看 src/lru.ts"
        );
    }

    #[test]
    fn 参数里含美元符号不会被二次替换() {
        // 链式 replace 的经典 bug：第一次把 $ARGUMENTS 换成含 $1 的内容，
        // 第二次又把那个 $1 换掉。让技能去做正则替换时一定会撞上。
        let out = substitute("把 $ARGUMENTS 应用一遍", r"s/(\w+)/$1_x/");
        assert_eq!(out, r"把 s/(\w+)/$1_x/ 应用一遍");
        // 路径形式同样成立
        assert_eq!(substitute("$ARGUMENTS", "src/$1.ts"), "src/$1.ts");
    }

    #[test]
    fn 越界的编号变空串而不是留着占位符() {
        // 留着 `$7` 的话模型会当成字面内容，然后在回复里问「$7 是什么」
        assert_eq!(substitute("[$1][$2][$7]", "只有一个"), "[只有一个][][]");
    }

    #[test]
    fn 孤立的美元符号原样保留() {
        // shell 变量、jQuery、正则里都很常见，不该被吃掉
        for text in ["价格是 $ 元", "echo $HOME", "$0 是脚本名", "$$", "末尾一个$"] {
            assert_eq!(substitute(text, "x"), text, "{text}");
        }
    }

    #[test]
    fn 没有参数时占位符全部清空() {
        assert_eq!(substitute("审查 $ARGUMENTS 的 $1", ""), "审查  的 ");
    }

    #[test]
    fn 多字节字符不会被切坏() {
        let body = "请审查「$1」这个文件，注意中文注释";
        assert_eq!(
            substitute(body, "记忆模块"),
            "请审查「记忆模块」这个文件，注意中文注释"
        );
        // 纯中文且不含 $ 时应当原样返回
        let plain = "全部都是中文，没有任何占位符。";
        assert_eq!(substitute(plain, "x"), plain);
    }

    #[test]
    fn 能看出一个命令要不要参数() {
        assert!(!placeholders("固定的一段指令").takes_args());
        let p = placeholders("看 $1 和 $3");
        assert_eq!(p.max_positional, 3);
        assert!(!p.arguments);
        assert!(p.takes_args());
        assert!(placeholders("处理 $ARGUMENTS").arguments);
    }
}
