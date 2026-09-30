# Upgrade to token-only monitoring

The model declaration audit, automatic collector, relay, configuration manager, and standalone probe have been removed. This version only monitors local token logs. It does not start a helper, send model requests, or read/write Codex authentication or routing configuration.

## Before replacing an older installation

An older client may still be using its collector's loopback URL. Removing the monitor does not change URLs already cached by running Codex clients.

1. While the older monitor is still available, disable automatic collection and use its **恢复直连配置** action. This restores only the fields it manages, preserving unrelated edits.
2. Finish any in-flight requests, restart every Codex client using that route, and confirm direct access works.
3. Use **已重启 Codex，停止转发** in the older monitor, then exit it and install this version.

If the older application is unavailable, recover the matching old version from Git history and use its recovery controls. Do not delete or replace the whole `config.toml` or guess a provider URL. A leftover loopback address may belong to another application.

## Existing data

Session logs, the session index, credentials, and previous diagnostic files are not deleted or rewritten by this version. Old collector preferences, journals, and captures remain untouched, and no longer have a runtime consumer. Keep recovery records until direct routing has been verified. Legacy diagnostic output directories remain ignored by Git to avoid accidentally publishing private metadata.

Existing project, conversation, date, and column-width preferences are preserved where valid. Obsolete column-width entries are ignored.

The previous implementation remains available in repository history at `c84769832d27234af9bae99eccdd1581580ebfa2`; migration does not require keeping it in the new application's source or binary.
