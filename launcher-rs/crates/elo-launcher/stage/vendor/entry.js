// What the stage takes from three.js and three-vrm, by name, so the bundle
// carries only these. `build.sh` turns this into ../vendor.js.
export {
  WebGLRenderer, Scene, PerspectiveCamera, DirectionalLight, HemisphereLight,
  Clock, Object3D, Vector3, Box3, MathUtils, SRGBColorSpace, ACESFilmicToneMapping,
  // A rigged glTF that is not a VRM (`avatar.js` GltfAvatar): its face
  // overlays, its pose, and its own animation clips.
  Mesh, PlaneGeometry, MeshStandardMaterial, MeshBasicMaterial, CanvasTexture, Quaternion,
  Color, AnimationMixer, LoopRepeat,
} from 'three';
export { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
export {
  VRMLoaderPlugin, VRMUtils, VRMExpressionPresetName, VRMHumanBoneName,
} from './node_modules/@pixiv/three-vrm/lib/three-vrm.module.js';
