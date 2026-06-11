export function CrowsFootMarkers() {
  return (
    <svg
      aria-hidden="true"
      className="pointer-events-none absolute size-0 overflow-hidden"
    >
      <defs>
        <marker
          id="crowsfoot-one"
          markerHeight="16"
          markerUnits="strokeWidth"
          markerWidth="16"
          orient="auto"
          refX="8"
          refY="8"
          viewBox="0 0 16 16"
        >
          <path
            d="M5 2.5v11M9 2.5v11"
            fill="none"
            stroke="var(--primary)"
            strokeLinecap="round"
            strokeWidth="1.8"
          />
        </marker>
        <marker
          id="crowsfoot-zero-or-one"
          markerHeight="18"
          markerUnits="strokeWidth"
          markerWidth="20"
          orient="auto"
          refX="10"
          refY="9"
          viewBox="0 0 20 18"
        >
          <circle
            cx="6"
            cy="9"
            fill="var(--card)"
            r="3.2"
            stroke="var(--primary)"
            strokeWidth="1.6"
          />
          <path
            d="M12 3.5v11"
            fill="none"
            stroke="var(--primary)"
            strokeLinecap="round"
            strokeWidth="1.8"
          />
        </marker>
        <marker
          id="crowsfoot-many"
          markerHeight="20"
          markerUnits="strokeWidth"
          markerWidth="24"
          orient="auto"
          refX="12"
          refY="10"
          viewBox="0 0 24 20"
        >
          <circle
            cx="6"
            cy="10"
            fill="var(--card)"
            r="3"
            stroke="var(--primary)"
            strokeWidth="1.5"
          />
          <path
            d="M12 10l8-6M12 10l8 6M12 10h8"
            fill="none"
            stroke="var(--primary)"
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth="1.8"
          />
        </marker>
        <marker
          id="crowsfoot-unknown"
          markerHeight="18"
          markerUnits="strokeWidth"
          markerWidth="18"
          orient="auto"
          refX="9"
          refY="9"
          viewBox="0 0 18 18"
        >
          <rect
            fill="var(--card)"
            height="6.4"
            stroke="var(--primary)"
            strokeWidth="1.5"
            transform="rotate(45 9 9)"
            width="6.4"
            x="5.8"
            y="5.8"
          />
        </marker>
        <marker
          id="crowsfoot-one-start"
          markerHeight="16"
          markerUnits="strokeWidth"
          markerWidth="16"
          orient="auto-start-reverse"
          refX="8"
          refY="8"
          viewBox="0 0 16 16"
        >
          <path
            d="M5 2.5v11M9 2.5v11"
            fill="none"
            stroke="var(--primary)"
            strokeLinecap="round"
            strokeWidth="1.8"
          />
        </marker>
        <marker
          id="crowsfoot-many-start"
          markerHeight="20"
          markerUnits="strokeWidth"
          markerWidth="24"
          orient="auto-start-reverse"
          refX="12"
          refY="10"
          viewBox="0 0 24 20"
        >
          <circle
            cx="6"
            cy="10"
            fill="var(--card)"
            r="3"
            stroke="var(--primary)"
            strokeWidth="1.5"
          />
          <path
            d="M12 10l8-6M12 10l8 6M12 10h8"
            fill="none"
            stroke="var(--primary)"
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth="1.8"
          />
        </marker>
        <marker
          id="crowsfoot-unknown-start"
          markerHeight="18"
          markerUnits="strokeWidth"
          markerWidth="18"
          orient="auto-start-reverse"
          refX="9"
          refY="9"
          viewBox="0 0 18 18"
        >
          <rect
            fill="var(--card)"
            height="6.4"
            stroke="var(--primary)"
            strokeWidth="1.5"
            transform="rotate(45 9 9)"
            width="6.4"
            x="5.8"
            y="5.8"
          />
        </marker>
      </defs>
    </svg>
  );
}
