# Mobile archive files

Internal Tauri plugin used by the Rust save/open commands. iOS uses the Files
export picker and the system Open In menu. Android streams exports to the selected
document URI and opens ZIMs using ACTION_VIEW with a temporary read grant. Its
FileProvider exposes only the staged exports cache. No
JavaScript plugin commands or broad filesystem permissions are exposed.
Canceling export leaves the completed archive intact.

The load hook disables pinch zoom in the app webview on both platforms.
