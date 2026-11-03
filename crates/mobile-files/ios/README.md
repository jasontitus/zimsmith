# iOS archive handoff

The native plugin presents a Files export picker and an Open In menu for completed
ZIM archives. Only ZIM paths inside the app container are accepted. Export uses a
copy so the internal checkpoint and archive remain available after cancellation
or after a viewer opens the file. UI operations run on the main queue.
