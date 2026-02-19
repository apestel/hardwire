export interface AdminUser {
	id: number;
	email: string;
	google_id: string;
	created_at: number;
}

export interface AuthResponse {
	token: string;
	user: AdminUser;
}

export interface DownloadStats {
	total_downloads: number;
	total_size: number;
	completed_downloads: number;
	average_download_time: number | null;
	success_rate: number;
}

export interface PeriodData {
	date: string;
	count: number;
	size: number;
}

export interface DownloadsByPeriod {
	period: string;
	data: PeriodData[];
}

export interface DownloadRecord {
	id: number;
	file_path: string;
	ip_address: string;
	transaction_id: string;
	status: string;
	file_size: number | null;
	started_at: number;
	finished_at: number | null;
}

export interface StatusDistribution {
	status: string;
	count: number;
	percentage: number;
}

export interface FileInfo {
	name: string;
	full_path: string;
	is_dir: boolean;
	size?: number;
	modified_at?: number;
	created_at?: number;
	children?: FileInfo[];
}

export interface SharedLinkResponse {
	id: string;
	url: string;
	expires_at: number | null;
}

export interface Task {
	id: string;
	status: 'Pending' | 'Running' | 'Completed' | 'Failed';
	created_at: number;
	started_at: number | null;
	finished_at: number | null;
	error: string | null;
	progress: number;
	archive_path: string | null;
}

export interface CreateTaskResponse {
	task_id: string;
}

// Matches Rust: #[serde(tag = "type", content = "data")]
export interface CreateArchiveInput {
	type: 'CreateArchive';
	data: {
		files?: string[];
		directory?: string;
		password?: string;
		output_path: string;
	};
}
