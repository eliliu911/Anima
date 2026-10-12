/**
 * 「这条 key 存在哪儿」的文案。
 *
 * 拆成纯函数是因为**这三种来源的可改性不一样**，写错了用户会在设置里
 * 白改半天：环境变量那条在界面里改不掉（要改 `.env.local` 再重启），
 * 另外两条能改。
 */
export type KeySource = "env" | "file" | "keychain";

/** 状态胶囊上那几个字 */
export function keySourceLabel(source: KeySource | null | undefined): string {
  switch (source) {
    case "env":
      return "环境变量";
    case "file":
      return "本机安全目录";
    case "keychain":
      return "钥匙串（旧）";
    default:
      return "";
  }
}

/** 输入框下面那行说明 */
export function keySourceHint(source: KeySource | null | undefined): string {
  switch (source) {
    case "env":
      return "由 .env.local 的环境变量注入，在这里改不掉 —— 要改就改那个文件再重启。";
    case "file":
      return "存在应用数据目录的 secrets.env，权限 600。打包版双击启动也读得到。";
    case "keychain":
      return "存在 macOS 钥匙串。重新保存一次会改存到本机安全目录，之后不再弹授权框。";
    default:
      return "保存后写进应用数据目录的 secrets.env（权限 600），不会弹授权框。";
  }
}

/** 能不能在界面里改 —— 只有环境变量那条不能 */
export function keyEditable(source: KeySource | null | undefined): boolean {
  return source !== "env";
}
