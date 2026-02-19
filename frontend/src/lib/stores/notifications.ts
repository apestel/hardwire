import { writable } from 'svelte/store';

export type NotificationKind = 'info' | 'success' | 'error' | 'progress';

export interface Notification {
	id: string;
	kind: NotificationKind;
	message: string;
	progress?: number;
	taskId?: string;
	dismissible: boolean;
	autoDismissMs?: number;
}

function createNotificationStore() {
	const { subscribe, update } = writable<Notification[]>([]);

	const store = {
		subscribe,
		add(n: Omit<Notification, 'id'>): string {
			const id = crypto.randomUUID();
			update((ns) => [...ns, { ...n, id }]);
			if (n.autoDismissMs) {
				setTimeout(() => store.dismiss(id), n.autoDismissMs);
			}
			return id;
		},
		dismiss(id: string) {
			update((ns) => ns.filter((n) => n.id !== id));
		},
		updateProgress(id: string, progress: number, message?: string) {
			update((ns) =>
				ns.map((n) =>
					n.id === id ? { ...n, progress, ...(message ? { message } : {}) } : n,
				),
			);
		},
		complete(id: string, message: string) {
			update((ns) =>
				ns.map((n) =>
					n.id === id
						? { ...n, kind: 'success' as NotificationKind, message, progress: undefined }
						: n,
				),
			);
			setTimeout(() => store.dismiss(id), 5000);
		},
		error(id: string, message: string) {
			update((ns) =>
				ns.map((n) =>
					n.id === id
						? { ...n, kind: 'error' as NotificationKind, message, progress: undefined }
						: n,
				),
			);
		},
	};

	return store;
}

export const notifications = createNotificationStore();
