import React, { useId } from 'react';

interface ThroughputSparklineProps {
  history: number[];
  className?: string;
  width?: number;
  height?: number;
}

export const ThroughputSparkline: React.FC<ThroughputSparklineProps> = ({
  history,
  className = 'w-16 h-5',
  width = 64,
  height = 20
}) => {
  const gradId = useId();
  if (!history || history.length < 2) return null;

  const max = Math.max(...history, 1);
  const points = history.map((val, idx) => {
    const x = (idx / (history.length - 1)) * width;
    const y = height - (val / max) * (height - 4) - 2;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });

  const pathD = `M ${points.join(' L ')}`;
  const areaD = `M ${points[0]} L ${points.join(' L ')} L ${width},${height} L 0,${height} Z`;

  return (
    <svg
      viewBox={`0 0 ${width} ${height}`}
      className={className}
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--argon-accent, #3B82F6)" stopOpacity="0.35" />
          <stop offset="100%" stopColor="var(--argon-accent, #3B82F6)" stopOpacity="0.0" />
        </linearGradient>
      </defs>
      <path d={areaD} fill={`url(#${gradId})`} />
      <path
        d={pathD}
        stroke="var(--argon-accent, #3B82F6)"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
};
