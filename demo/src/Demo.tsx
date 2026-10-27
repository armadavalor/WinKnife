import {AbsoluteFill, Easing, Img, interpolate, staticFile, useCurrentFrame} from "remotion";
import {linearTiming, TransitionSeries} from "@remotion/transitions";
import {fade} from "@remotion/transitions/fade";
import {CameraMotionBlur} from "@remotion/motion-blur";
import {KnifeLogo} from "./KnifeLogo";
import {LoopingSky} from "./SkyBackdrop";

// A ~14.5 s looping b-roll for the top of the README. The stills are real 2x
// captures from a throwaway CI profile (demo/README.md), taken with the app's
// backdrop hidden so they sit over the same sky, animated here (SkyBackdrop.tsx).
const SHOT = 75;
const TOUR = 105;
const END = 90;
const XFADE = 12;

type Move = {scale: [number, number]; x: [number, number]; y: [number, number]; rx: [number, number]; ry: [number, number]};

const SHOTS: {img: string; next?: string; caption?: string; move: Move}[] = [
  {img: "01-dashboard.png", move: {scale: [1.0, 1.08], x: [0, 0], y: [10, -6], rx: [8, 3], ry: [-6, 0]}},
  {
    img: "02-cleaner.png",
    caption: "Every suggestion says why.",
    move: {scale: [1.18, 1.26], x: [70, 30], y: [60, 20], rx: [0, 0], ry: [4, -2]},
  },
  {
    img: "03-quarantine.png",
    next: "04-restored.png",
    caption: "Quarantine first. Restore in one click.",
    move: {scale: [1.1, 1.2], x: [-40, -90], y: [0, 10], rx: [2, 0], ry: [0, -5]},
  },
  {img: "06-treemap.png", move: {scale: [1.02, 1.12], x: [0, 0], y: [0, 0], rx: [6, 0], ry: [-4, 3]}},
];

// One still per tool, whipped past with camera motion blur.
const TOUR_STILLS = [
  "duplicates", "tweaks", "legacy-panels", "network", "scheduled-scan", "repair",
  "archiver", "shutdown-timer", "keep-awake", "restore-point", "settings",
].map((route) => `tour-${route}.png`);

export const DURATION = SHOTS.length * SHOT + TOUR + END - (SHOTS.length + 1) * XFADE;

const BG = "#0d0e11";
const FG = "#eceef2";
const MUTED = "#9aa3b2";
const ACCENT = "#19c3d6";
const font = "Segoe UI, system-ui, sans-serif";
const ease = Easing.bezier(0.16, 1, 0.3, 1);
const soft = Easing.bezier(0.45, 0, 0.55, 1);
const clamp = {extrapolateLeft: "clamp", extrapolateRight: "clamp"} as const;

const WIN_W = 1040;
const WIN_H = 650;

const lerp = (f: number, [a, b]: [number, number]) => interpolate(f, [0, SHOT], [a, b], {...clamp, easing: soft});

const Caption = ({text, length = SHOT}: {text: string; length?: number}) => {
  const f = useCurrentFrame();
  const a = interpolate(f, [14, 28, length - 6, length], [0, 1, 1, 0], {...clamp, easing: ease});
  return (
    <div
      style={{
        position: "absolute",
        left: 64,
        bottom: 48,
        padding: "12px 22px",
        borderRadius: 999,
        background: "#0d0e11d9",
        border: "1px solid #ffffff1a",
        color: FG,
        fontFamily: font,
        fontSize: 28,
        fontWeight: 600,
        opacity: a,
        translate: `0 ${(1 - a) * 14}px`,
      }}
    >
      {text}
    </div>
  );
};

const Shot = ({shot}: {shot: (typeof SHOTS)[number]}) => {
  const f = useCurrentFrame();
  const {move} = shot;
  const swap = shot.next ? interpolate(f, [SHOT * 0.5, SHOT * 0.5 + 10], [0, 1], clamp) : 0;
  return (
    <AbsoluteFill>
      <AbsoluteFill style={{alignItems: "center", justifyContent: "center", perspective: 1800}}>
        <div
          style={{
            position: "relative",
            width: WIN_W,
            height: WIN_H,
            borderRadius: 14,
            overflow: "hidden",
            border: "1px solid #ffffff1f",
            boxShadow: "0 30px 80px #000c, 0 0 0 1px #0006",
            scale: String(lerp(f, move.scale)),
            translate: `${lerp(f, move.x)}px ${lerp(f, move.y)}px`,
            transform: `rotateX(${lerp(f, move.rx)}deg) rotateY(${lerp(f, move.ry)}deg)`,
          }}
        >
          <Img src={staticFile(`broll/${shot.img}`)} style={{position: "absolute", width: WIN_W, height: WIN_H}} />
          {shot.next ? (
            <Img
              src={staticFile(`broll/${shot.next}`)}
              style={{position: "absolute", width: WIN_W, height: WIN_H, opacity: swap}}
            />
          ) : null}
        </div>
      </AbsoluteFill>
      {shot.caption ? <Caption text={shot.caption} /> : null}
    </AbsoluteFill>
  );
};

// Scenes are see-through over the sky, so the outgoing one has to fade too.
const xfade = fade({shouldFadeOutExitingScene: true});

const GAP = 70;
const STEP = WIN_W + GAP;

// Each still holds for a beat, then the strip whips to the next one.
const tourPos = (f: number) => {
  const per = TOUR / TOUR_STILLS.length;
  const i = Math.floor(f / per);
  const local = interpolate(f - i * per, [per * 0.35, per], [0, 1], {...clamp, easing: soft});
  return Math.min(i + local, TOUR_STILLS.length - 1);
};

const Tour = () => {
  const f = useCurrentFrame();
  const pull = interpolate(f, [0, 20, TOUR - 20, TOUR], [1, 0.82, 0.82, 1], {...clamp, easing: soft});
  return (
    <AbsoluteFill>
      <CameraMotionBlur shutterAngle={240} samples={10}>
        <AbsoluteFill style={{alignItems: "center", justifyContent: "center"}}>
          <div style={{position: "relative", width: WIN_W, height: WIN_H, scale: String(pull)}}>
            {TOUR_STILLS.map((img, i) => (
              <div
                key={img}
                style={{
                  position: "absolute",
                  left: (i - tourPos(f)) * STEP,
                  width: WIN_W,
                  height: WIN_H,
                  borderRadius: 14,
                  overflow: "hidden",
                  border: "1px solid #ffffff1f",
                  boxShadow: "0 30px 80px #000c",
                }}
              >
                <Img src={staticFile(`broll/${img}`)} style={{width: WIN_W, height: WIN_H}} />
              </div>
            ))}
          </div>
        </AbsoluteFill>
      </CameraMotionBlur>
      <Caption text="Every tool in one place." length={TOUR} />
    </AbsoluteFill>
  );
};

const SLOGAN = ["Fast.", "Modular.", "Useful."];

const EndCard = () => {
  const f = useCurrentFrame();
  const appear = interpolate(f, [0, 14], [0, 1], {...clamp, easing: ease});
  // The blade swings open, like the app's knife does on load.
  const open = interpolate(f, [8, 34], [0, 1], {...clamp, easing: Easing.bezier(0.34, 1.4, 0.64, 1)});
  // Fade to the bare sky so the loop back to the first shot is a dissolve, not a cut.
  const out = interpolate(f, [END - 16, END], [1, 0], {...clamp, easing: soft});
  return (
    <AbsoluteFill>
      <AbsoluteFill style={{alignItems: "center", justifyContent: "center", fontFamily: font, opacity: out}}>
        <div style={{opacity: appear, scale: String(0.85 + appear * 0.15)}}>
          <KnifeLogo size={200} open={open} />
        </div>
        <div style={{marginTop: 30, display: "flex", gap: 16, fontSize: 40, fontWeight: 700}}>
          {SLOGAN.map((t, i) => {
            const a = interpolate(f, [30 + i * 7, 44 + i * 7], [0, 1], {...clamp, easing: ease});
            return (
              <span key={t} style={{color: i === 2 ? ACCENT : FG, opacity: a, translate: `0 ${(1 - a) * 12}px`}}>
                {t}
              </span>
            );
          })}
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

// The first shot fades in over the sky the end card fades out to.
const FadeIn = ({children}: {children: React.ReactNode}) => {
  const f = useCurrentFrame();
  return <AbsoluteFill style={{opacity: interpolate(f, [0, 12], [0, 1], clamp)}}>{children}</AbsoluteFill>;
};

export const Demo = () => {
  const items: React.ReactNode[] = [];
  SHOTS.forEach((shot, i) => {
    items.push(
      <TransitionSeries.Sequence key={shot.img} durationInFrames={SHOT}>
        {i === 0 ? (
          <FadeIn>
            <Shot shot={shot} />
          </FadeIn>
        ) : (
          <Shot shot={shot} />
        )}
      </TransitionSeries.Sequence>,
      <TransitionSeries.Transition key={`t${i}`} presentation={xfade} timing={linearTiming({durationInFrames: XFADE})} />,
    );
  });
  return (
    <AbsoluteFill style={{backgroundColor: BG}}>
      <LoopingSky duration={DURATION} />
      <TransitionSeries>
        {items}
        <TransitionSeries.Sequence durationInFrames={TOUR}>
          <Tour />
        </TransitionSeries.Sequence>
        <TransitionSeries.Transition presentation={xfade} timing={linearTiming({durationInFrames: XFADE})} />
        <TransitionSeries.Sequence durationInFrames={END}>
          <EndCard />
        </TransitionSeries.Sequence>
      </TransitionSeries>
    </AbsoluteFill>
  );
};
