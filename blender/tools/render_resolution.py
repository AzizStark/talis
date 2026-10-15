"""Resolution/crop planning shared by offline bakes; no Blender dependency."""
import math

REFERENCE_SIZE = (2560, 1440)
REFERENCE_ARM_CROP = (1160, 585, 960, 630)


def validate_resolution(width, height):
    if not all(isinstance(value, int) and 1 <= value <= 16384 for value in (width, height)):
        raise ValueError('Resolution dimensions must be integers from 1 to 16384')


def pixel_grid(width, height):
    """Integer pixels whose border fractions are exactly representable in binary.

    1440p retains the existing 20x45 grid. At 4K the old vertical step was
    67.5 pixels; use 135 instead, avoiding truncation and one-pixel seams.
    """
    validate_resolution(width, height)
    return width // math.gcd(width, 128), height // math.gcd(height, 32)


def button_crop(width, height, rectangles):
    """Quantize button/shadow bounds and reject runtime-unsupported atlas sizes."""
    validate_resolution(width, height)
    if not rectangles or any(len(box) != 4 or not all(math.isfinite(v) for v in box)
                             or not 0 <= box[0] < box[2] <= 1
                             or not 0 <= box[1] < box[3] <= 1 for box in rectangles):
        raise ValueError('Button bounds must be finite scene-normalized rectangles')
    gx, gy = (32, 32) if (width, height) == REFERENCE_SIZE else pixel_grid(width, height)
    left = math.floor(min(box[0] for box in rectangles) * width / gx) * gx
    right = math.ceil(max(box[2] for box in rectangles) * width / gx) * gx
    top = math.floor(min(box[1] for box in rectangles) * height / gy) * gy
    bottom = math.ceil(max(box[3] for box in rectangles) * height / gy) * gy
    if not 0 < right-left <= 512 or not 0 < bottom-top <= 256:
        raise ValueError('Projected button atlas exceeds runtime 512x256 limits; review camera/control geometry')
    return left, top, right-left, bottom-top


def arm_crop(width, height, explicit=None):
    gx, gy = pixel_grid(width, height)
    if explicit is None:
        x, y, w, h = REFERENCE_ARM_CROP
        left = math.floor(x * width / REFERENCE_SIZE[0] / gx) * gx
        right = math.ceil((x + w) * width / REFERENCE_SIZE[0] / gx) * gx
        top = math.floor(y * height / REFERENCE_SIZE[1] / gy) * gy
        bottom = math.ceil((y + h) * height / REFERENCE_SIZE[1] / gy) * gy
        values = (left, top, right - left, bottom - top)
    else:
        try:
            values = tuple(int(value) for value in explicit.split(','))
        except ValueError as error:
            raise ValueError('Crop requires x,y,width,height integer pixels') from error
        if len(values) != 4:
            raise ValueError('Crop requires x,y,width,height integer pixels')
    x, y, w, h = values
    if (min(x, y) < 0 or min(w, h) < 1 or x + w > width or y + h > height
            or x % gx or (x + w) % gx or y % gy or (y + h) % gy):
        raise ValueError('Crop must fit the image and use its exact integer border grid')
    return values
