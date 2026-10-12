/**
 * VS Code Dark+ / GitHub Dark 风格的高性能轻量语法高亮分词器
 */

const TS_KEYWORDS = new Set([
  "import", "from", "export", "default", "interface", "type", "const", "let", "var",
  "function", "class", "return", "if", "else", "for", "while", "do", "switch", "case",
  "break", "continue", "async", "await", "try", "catch", "finally", "throw", "new",
  "typeof", "instanceof", "in", "of", "as", "is", "extends", "implements", "readonly",
  "private", "public", "protected", "static", "abstract", "override", "void", "any",
  "never", "unknown", "enum", "true", "false", "null", "undefined", "this", "super"
]);

const RS_KEYWORDS = new Set([
  "pub", "fn", "struct", "enum", "impl", "trait", "use", "mod", "let", "mut", "const",
  "static", "type", "return", "if", "else", "match", "loop", "while", "for", "in",
  "break", "continue", "async", "await", "move", "ref", "self", "Self", "super", "crate",
  "where", "as", "unsafe", "dyn", "true", "false", "Some", "None", "Ok", "Err"
]);

const PY_KEYWORDS = new Set([
  "def", "class", "import", "from", "as", "return", "if", "elif", "else", "for", "while",
  "break", "continue", "try", "except", "finally", "raise", "with", "yield", "async",
  "await", "lambda", "global", "nonlocal", "pass", "True", "False", "None", "self", "in",
  "is", "and", "or", "not"
]);

export interface HighlightToken {
  text: string;
  type: "keyword" | "string" | "comment" | "type" | "function" | "number" | "tag" | "punct" | "plain";
}

export function tokenizeLine(line: string, ext: string): HighlightToken[] {
  const tokens: HighlightToken[] = [];
  let i = 0;
  const len = line.length;

  const keywords = ext === "rs" ? RS_KEYWORDS : ext === "py" ? PY_KEYWORDS : TS_KEYWORDS;

  while (i < len) {
    // 1. 单行注释 // 或 #
    if ((line[i] === "/" && line[i + 1] === "/") || (ext === "py" && line[i] === "#")) {
      tokens.push({ text: line.slice(i), type: "comment" });
      break;
    }

    // 2. 字符串: ", ', `
    if (line[i] === '"' || line[i] === "'" || line[i] === "`") {
      const quote = line[i];
      let j = i + 1;
      let escaped = false;
      while (j < len) {
        if (line[j] === "\\" && !escaped) {
          escaped = true;
          j++;
          continue;
        }
        if (line[j] === quote && !escaped) {
          j++;
          break;
        }
        escaped = false;
        j++;
      }
      tokens.push({ text: line.slice(i, j), type: "string" });
      i = j;
      continue;
    }

    // 3. 数字字面量
    if (/\d/.test(line[i]) && (i === 0 || /[\s,([{:;=+\-*/<>]/.test(line[i - 1]))) {
      let j = i;
      while (j < len && /[\d.a-fA-Fx_]/.test(line[j])) {
        j++;
      }
      tokens.push({ text: line.slice(i, j), type: "number" });
      i = j;
      continue;
    }

    // 4. JSX/HTML 标签: <div, </button, <span, etc.
    if ((ext === "tsx" || ext === "jsx" || ext === "html") && line[i] === "<" && (i + 1 < len && /[a-zA-Z/]/.test(line[i + 1]))) {
      let j = i + 1;
      if (line[j] === "/") j++;
      while (j < len && /[a-zA-Z0-9_-]/.test(line[j])) j++;
      tokens.push({ text: line.slice(i, j), type: "tag" });
      i = j;
      continue;
    }

    // 5. 标识符 / 关键字 / 类型 / 函数名
    if (/[a-zA-Z_$]/.test(line[i])) {
      let j = i;
      while (j < len && /[a-zA-Z0-9_$]/.test(line[j])) {
        j++;
      }
      const word = line.slice(i, j);

      if (keywords.has(word)) {
        tokens.push({ text: word, type: "keyword" });
      } else if (j < len && line[j] === "(") {
        tokens.push({ text: word, type: "function" });
      } else if (/^[A-Z]/.test(word)) {
        tokens.push({ text: word, type: "type" });
      } else {
        tokens.push({ text: word, type: "plain" });
      }
      i = j;
      continue;
    }

    // 6. 标点符号与空白
    tokens.push({ text: line[i], type: "punct" });
    i++;
  }

  return tokens;
}

export function HighlightedCodeLine({ line, ext }: { line: string; ext: string }) {
  const tokens = tokenizeLine(line, ext);

  return (
    <span className="font-mono text-[12px] leading-[1.65]">
      {tokens.map((tok, idx) => {
        let colorClass = "text-[#24292f] dark:text-[#e6edf3]"; // 默认纯黑/亮灰

        switch (tok.type) {
          case "keyword":
            colorClass = "text-[#cf222e] dark:text-[#ff7b72] font-semibold"; // 关键字鲜红/玫红
            break;
          case "string":
            colorClass = "text-[#0a3069] dark:text-[#a5d6ff]"; // 字符串深海蓝/亮青蓝
            break;
          case "comment":
            colorClass = "text-[#57606a] dark:text-[#8b949e] italic"; // 注释清晰深灰/暗灰
            break;
          case "type":
            colorClass = "text-[#953800] dark:text-[#ffa657] font-medium"; // 类型/类名深橙/琥珀黄
            break;
          case "function":
            colorClass = "text-[#8250df] dark:text-[#d2a8ff] font-medium"; // 函数调用明亮紫
            break;
          case "number":
            colorClass = "text-[#0550ae] dark:text-[#79c0ff]"; // 数字深蓝/青蓝
            break;
          case "tag":
            colorClass = "text-[#116329] dark:text-[#7ee787]"; // JSX标签深绿/青绿
            break;
          case "punct":
            colorClass = "text-[#57606a] dark:text-[#8b949e]"; // 标点深灰
            break;
          default:
            colorClass = "text-[#24292f] dark:text-[#e6edf3]";
        }

        return (
          <span key={idx} className={colorClass}>
            {tok.text}
          </span>
        );
      })}
    </span>
  );
}
