/**
 * Every object kind the console has, registered by the module that defines it.
 *
 * Each file in this folder registers its kinds when imported, and this one imports
 * the folder with a glob — so a new kind is a new file beside the others, and no
 * list here or anywhere else names them. The root layout imports this once.
 */

import.meta.glob('./*.kind.ts', { eager: true });
