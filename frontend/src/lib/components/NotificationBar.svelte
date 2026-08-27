<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { notifications } from '$lib/stores/notifications';
	import { getToken } from '$lib/auth';
	import Notification from './Notification.svelte';

	let ws: WebSocket | null = null;
	let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
	let reconnectDelay = 1000;
	let destroyed = false;

	onMount(() => {
		connectWs();
	});

	onDestroy(() => {
		destroyed = true;
		if (reconnectTimer) clearTimeout(reconnectTimer);
		ws?.close();
	});

	function connectWs() {
		if (destroyed) return;
		const token = getToken();
		// Logged out (or logout happened while a reconnect was pending): stop.
		if (!token) return;

		const proto = location.protocol === 'https:' ? 'wss' : 'ws';
		// The token is sent as the first message after the socket opens, so it
		// never appears in the URL / access logs.
		ws = new WebSocket(`${proto}://${location.host}/admin/live_update`);

		ws.onopen = () => {
			ws?.send(token);
			reconnectDelay = 1000;
		};

		ws.onmessage = (event) => {
			try {
				const msg = JSON.parse(event.data);
				if (msg.event === 'download_progress') {
					const pct = msg.file_size > 0
						? Math.round(((msg.start_offset + msg.read_bytes) / msg.file_size) * 100)
						: Math.round((msg.read_bytes / msg.chunk_bytes) * 100);
					const filename = (msg.file_path as string).split('/').pop() ?? msg.file_path;
					notifications.downloadProgress(msg.transaction_id, filename, pct);
				}
			} catch {
				// ignore malformed messages
			}
		};

		ws.onclose = (event) => {
			ws = null;
			if (destroyed) return;
			// 4401 = rejected by the server (invalid/expired token): retrying
			// with the same token is pointless. Same if the user logged out.
			if (event.code === 4401 || !getToken()) return;
			// Exponential backoff, capped at 30s.
			reconnectTimer = setTimeout(connectWs, reconnectDelay);
			reconnectDelay = Math.min(reconnectDelay * 2, 30_000);
		};
	}
</script>

<div class="fixed top-4 right-4 z-50 flex flex-col gap-2 w-80 pointer-events-none">
	{#each $notifications as notif (notif.id)}
		<div class="pointer-events-auto">
			<Notification {notif} ondismiss={() => notifications.dismiss(notif.id)} />
		</div>
	{/each}
</div>
