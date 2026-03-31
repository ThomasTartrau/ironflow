interface TimeAgoProps {
	date: string;
}

export function TimeAgo({ date }: TimeAgoProps) {
	const getTimeAgo = (isoDate: string): string => {
		const now = new Date();
		const then = new Date(isoDate);
		const seconds = Math.floor((now.getTime() - then.getTime()) / 1000);

		if (seconds < 60) return "just now";
		if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
		if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
		if (seconds < 604800) return `${Math.floor(seconds / 86400)}d ago`;

		return then.toLocaleDateString();
	};

	return <span>{getTimeAgo(date)}</span>;
}
