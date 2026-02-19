<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { notifications } from '$lib/stores/notifications';
	import { getToken } from '$lib/auth';
	import Notification from './Notification.svelte';

	let ws: WebSocket | null = null;
	let wsNotifIds: Map<string, string> = new Map(); // transaction_id → notif id

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
				const data = JSON.parse(event.data);
				if (data.event === 'download_progress') {
					const txId: string = data.transaction_id;
					const pct = Math.round((data.read_bytes / data.total_bytes) * 100);
					const filename = (data.file_path as string).split('/').pop() ?? data.file_path;
					const msg = `Downloading ${filename} — ${pct}%`;

					if (!wsNotifIds.has(txId)) {
						const id = notifications.add({
							kind: 'progress',
							message: msg,
							progress: pct,
							dismissible: false,
						});
						wsNotifIds.set(txId, id);
					} else {
						const id = wsNotifIds.get(txId)!;
						if (pct >= 100) {
							notifications.complete(id, `Download complete: ${filename}`);
							wsNotifIds.delete(txId);
						} else {
							notifications.updateProgress(id, pct, msg);
						}
					}
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
