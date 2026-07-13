function point(x, y) {
    return Object.freeze({ x, y });
}

export const GAMEPAD_LOWER_CUTOUT = Object.freeze({
    left: Object.freeze([
        point(246, 370),
        point(267, 365),
        point(298, 360),
        point(332, 360),
    ]),
    right: Object.freeze([
        point(468, 360),
        point(502, 360),
        point(533, 365),
        point(554, 370),
    ]),
});

const [leftStart, leftControlA, leftControlB, leftEnd] = GAMEPAD_LOWER_CUTOUT.left;
const [rightStart, rightControlA, rightControlB, rightEnd] = GAMEPAD_LOWER_CUTOUT.right;

export const GAMEPAD_BODY_PATH = `M270 82 C231 81 193 91 160 111 C116 138 91 184 77 244 L43 390 C32 438 58 479 101 487 C132 493 158 480 180 452 L${leftStart.x} ${leftStart.y} C${leftControlA.x} ${leftControlA.y} ${leftControlB.x} ${leftControlB.y} ${leftEnd.x} ${leftEnd.y} H${rightStart.x} C${rightControlA.x} ${rightControlA.y} ${rightControlB.x} ${rightControlB.y} ${rightEnd.x} ${rightEnd.y} L620 452 C642 480 668 493 699 487 C742 479 768 438 757 390 L723 244 C709 184 684 138 640 111 C607 91 569 81 530 82 C493 67 450 60 400 60 C350 60 307 67 270 82 Z`;

export function wellClearsGamepadCutout({ x, y, radius }, clearance = 0) {
    const checkedRadius = radius + clearance;
    for (let degree = 0; degree <= 180; degree += 1) {
        const angle = degree * Math.PI / 180;
        const sampleX = x + Math.cos(angle) * checkedRadius;
        const sampleY = y + Math.sin(angle) * checkedRadius;
        const cutoutY = lowerCutoutY(sampleX);
        if (cutoutY !== null && sampleY > cutoutY) return false;
    }
    return true;
}

function lowerCutoutY(x) {
    if (x < leftStart.x || x > rightEnd.x) return null;
    if (x <= leftEnd.x) return cubicYAtX(GAMEPAD_LOWER_CUTOUT.left, x);
    if (x < rightStart.x) return leftEnd.y;
    return cubicYAtX(GAMEPAD_LOWER_CUTOUT.right, x);
}

function cubicYAtX(points, x) {
    let low = 0;
    let high = 1;
    for (let iteration = 0; iteration < 32; iteration += 1) {
        const middle = (low + high) / 2;
        if (cubicCoordinate(points, middle, 'x') < x) low = middle;
        if (cubicCoordinate(points, middle, 'x') >= x) high = middle;
    }
    return cubicCoordinate(points, (low + high) / 2, 'y');
}

function cubicCoordinate(points, t, coordinate) {
    const inverse = 1 - t;
    return (inverse ** 3 * points[0][coordinate])
        + (3 * inverse ** 2 * t * points[1][coordinate])
        + (3 * inverse * t ** 2 * points[2][coordinate])
        + (t ** 3 * points[3][coordinate]);
}
