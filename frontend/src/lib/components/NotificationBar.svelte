<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { notifications } from '$lib/stores/notifications';
	import { getToken } from '$lib/auth';
	import Notification from './Notification.svelte';

	let ws: WebSocket | null = null;

	onMount(() => {
		connectWs();
	});

	onDestroy(() => {
		ws?.close();
	});

	function connectWs() {
		const token = getToken();
		if (!token) return;

		const proto = location.protocol === 'https:' ? 'wss' : 'ws';
		ws = new WebSocket(
			`${proto}://${location.host}/admin/live_update?token=${encodeURIComponent(token)}`,
		);

		ws.onmessage = (event) => {
			try {
				const msg = JSON.parse(event.data);
				if (msg.event === 'download_progress') {
					const pct = Math.round((msg.read_bytes / msg.total_bytes) * 100);
					const filename = (msg.file_path as string).split('/').pop() ?? msg.file_path;
					notifications.downloadProgress(msg.transaction_id, filename, pct);
				}
			} catch {
				// ignore malformed messages
			}
		};

		ws.onclose = () => {
			setTimeout(connectWs, 3000);
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
