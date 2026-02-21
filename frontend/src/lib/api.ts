import { getToken } from './auth';
import type {
	AuthResponse,
	CreateArchiveInput,
	CreateTaskResponse,
	DownloadRecord,
	DownloadsByPeriod,
	DownloadStats,
	FileInfo,
	SharedLinkResponse,
	StatusDistribution,
	Task,
} from './types';

function authHeaders(): Record<string, string> {
	const token = getToken();
	return token ? { Authorization: `Bearer ${token}` } : {};
}

async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
	const res = await fetch(path, {
		...init,
		headers: {
			'Content-Type': 'application/json',
			...authHeaders(),
			...(init?.headers ?? {}),
		},
	});
	if (!res.ok) {
		const err = await res.json().catch(() => ({ error: res.statusText }));
		throw new Error(err.error ?? 'API error');
	}
	if (res.status === 204) {
		return undefined as T;
	}
	return res.json() as Promise<T>;
}

export async function fetchAuthCallback(code: string, state: string): Promise<AuthResponse> {
	return apiFetch<AuthResponse>(
		`/admin/auth/google/callback?code=${encodeURIComponent(code)}&state=${encodeURIComponent(state)}`,
	);
}

export const fetchDownloadStats = () => apiFetch<DownloadStats>('/admin/api/stats/downloads');

export const fetchDownloadsByPeriod = (period: string, limit = 30) =>
	apiFetch<DownloadsByPeriod>(
		`/admin/api/stats/downloads/by_period?period=${period}&limit=${limit}`,
	);

export const fetchRecentDownloads = (limit = 100) =>
	apiFetch<DownloadRecord[]>(`/admin/api/stats/downloads/recent?limit=${limit}`);

export const fetchStatusDistribution = () =>
	apiFetch<StatusDistribution[]>('/admin/api/stats/downloads/status');

export const fetchFiles = () => apiFetch<FileInfo[]>('/admin/api/list_files');

export const rescanFiles = () =>
	apiFetch<void>('/admin/api/files/rescan', { method: 'POST' });

export const createSharedLink = (file_paths: string[], expires_at?: number) =>
	apiFetch<SharedLinkResponse>('/admin/api/create_shared_link', {
		method: 'POST',
		body: JSON.stringify({ file_paths, expires_at }),
	});

export const createTask = (input: CreateArchiveInput) =>
	apiFetch<CreateTaskResponse>('/admin/api/tasks', {
		method: 'POST',
		body: JSON.stringify(input),
	});

export const getTaskStatus = (task_id: string) =>
	apiFetch<Task>(`/admin/api/tasks/${task_id}`);

export const getTaskDownloadUrl = (task_id: string) => `/admin/api/tasks/${task_id}/download`;
