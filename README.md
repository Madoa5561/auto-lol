# LanePilot

Windows のタスクトレイに常駐し、起動中の League of Legends クライアントを補助する軽量 Rust アプリです。

## ダウンロードと使い方

1. [Releases](https://github.com/Madoa5561/auto-lol/releases/latest) から `LanePilot-v0.1.0-windows-x64.zip` をダウンロード
2. ZIP を好きな場所へ展開
3. League of Legends クライアントを起動
4. `LanePilot.exe` を起動して、ロールごとの `PICK` / `BAN` を設定
5. 「設定を保存」を押す

設定とチャンピオン画像は、`LanePilot.exe`と同じ場所に作られる`LanePilotData`フォルダーへ保存されます。更新時もこのフォルダーを残せば設定を引き継げます。

## 機能

- レディーチェックの自動承認
- TOP / JUNGLE / MID / ADC / SUPPORT ごとの優先ピック設定
- TOP / JUNGLE / MID / ADC / SUPPORT ごとの自動 BAN 設定
- BAN または他プレイヤーが選択済みのチャンピオンを除外し、次候補へ切り替え
- 自動ホバーと自動ロックインを個別に切り替え
- 検索付きアイコングリッドからチャンピオン候補を優先順に追加
- `LanePilotData\settings.json` への設定保存
- League クライアントの認証情報は保存・ログ出力しない

各ロールの `PICK` または `BAN` ボタンからチャンピオンを選びます。追加した順番が優先順になり、BAN・確定済みピックと重複した場合は次候補へ移ります。

チャンピオン画像は League クライアントから取得し、`LanePilotData\champion-icons` にキャッシュします。クライアントの認証情報はキャッシュに含まれません。

## ソースからビルド

Rust と Windows SDK が必要です。

```powershell
cargo build --release
```

生成物は `target\release\lane-pilot.exe` です。終了するときはタスクトレイのアイコンを右クリックして「終了」を選びます。

## 注意

League Client API は Riot Games による公式サポート対象ではなく、クライアント更新で動作しなくなる可能性があります。配布する場合は Riot Developer Portal への製品登録と、最新ポリシーへの適合確認が必要です。

自動操作機能の利用可否は地域・時期・運用形態によって判断が変わる可能性があります。自動ロックインは初期状態で無効です。

LanePilot は Riot Games の公式製品ではなく、Riot Games による承認・支援を受けたものではありません。
