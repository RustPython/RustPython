"""
Quadtree N-body benchmark using the pyperf framework.

This benchmark simulates gravitational interactions between particles using
a quadtree for spatial partitioning and the Barnes-Hut approximation algorithm.
No visualization, pure Python implementation without dependencies.
"""

import pyperf
import math

DEFAULT_ITERATIONS = 100
DEFAULT_PARTICLES = 200
DEFAULT_THETA = 0.5

# Constants
G = 6.67430e-11  # Gravitational constant
SOFTENING = 5.0  # Softening parameter to avoid singularities
TIME_STEP = 0.1  # Time step for simulation

class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

class Particle:
    def __init__(self, x, y, mass=1.0):
        self.position = Point(x, y)
        self.velocity = Point(0, 0)
        self.acceleration = Point(0, 0)
        self.mass = mass
    
    def update(self, time_step):
        # Update velocity using current acceleration
        self.velocity.x += self.acceleration.x * time_step
        self.velocity.y += self.acceleration.y * time_step
        
        # Update position using updated velocity
        self.position.x += self.velocity.x * time_step
        self.position.y += self.velocity.y * time_step
        
        # Reset acceleration for next frame
        self.acceleration.x = 0
        self.acceleration.y = 0
    
    def apply_force(self, fx, fy):
        # F = ma -> a = F/m
        self.acceleration.x += fx / self.mass
        self.acceleration.y += fy / self.mass

class Rectangle:
    def __init__(self, x, y, w, h):
        self.x = x
        self.y = y
        self.w = w
        self.h = h
    
    def contains(self, point):
        return (
            point.x >= self.x - self.w and
            point.x < self.x + self.w and
            point.y >= self.y - self.h and
            point.y < self.y + self.h
        )
    
    def intersects(self, range_rect):
        return not (
            range_rect.x - range_rect.w > self.x + self.w or
            range_rect.x + range_rect.w < self.x - self.w or
            range_rect.y - range_rect.h > self.y + self.h or
            range_rect.y + range_rect.h < self.y - self.h
        )

class QuadTree:
    def __init__(self, boundary, capacity=4):
        self.boundary = boundary
        self.capacity = capacity
        self.particles = []
        self.divided = False
        self.center_of_mass = Point(0, 0)
        self.total_mass = 0
        self.northeast = None
        self.northwest = None
        self.southeast = None
        self.southwest = None
    
    def insert(self, particle):
        if not self.boundary.contains(particle.position):
            return False
        
        if len(self.particles) < self.capacity and not self.divided:
            self.particles.append(particle)
            self.update_mass_distribution(particle)
            return True
        
        if not self.divided:
            self.subdivide()
        
        if self.northeast.insert(particle): return True
        if self.northwest.insert(particle): return True
        if self.southeast.insert(particle): return True
        if self.southwest.insert(particle): return True
        
        # This should never happen if the boundary check is correct
        return False
    
    def update_mass_distribution(self, particle):
        # Update center of mass and total mass when adding a particle
        total_mass_new = self.total_mass + particle.mass
        
        # Calculate new center of mass
        if total_mass_new > 0:
            self.center_of_mass.x = (self.center_of_mass.x * self.total_mass + 
                                    particle.position.x * particle.mass) / total_mass_new
            self.center_of_mass.y = (self.center_of_mass.y * self.total_mass + 
                                    particle.position.y * particle.mass) / total_mass_new
        
        self.total_mass = total_mass_new
    
    def subdivide(self):
        x = self.boundary.x
        y = self.boundary.y
        w = self.boundary.w / 2
        h = self.boundary.h / 2
        
        ne = Rectangle(x + w, y - h, w, h)
        self.northeast = QuadTree(ne, self.capacity)
        
        nw = Rectangle(x - w, y - h, w, h)
        self.northwest = QuadTree(nw, self.capacity)
        
        se = Rectangle(x + w, y + h, w, h)
        self.southeast = QuadTree(se, self.capacity)
        
        sw = Rectangle(x - w, y + h, w, h)
        self.southwest = QuadTree(sw, self.capacity)
        
        self.divided = True
        
        # Redistribute particles to children
        for particle in self.particles:
            self.northeast.insert(particle)
            self.northwest.insert(particle)
            self.southeast.insert(particle)
            self.southwest.insert(particle)
        
        # Clear the particles at this node
        self.particles = []
    
    def calculate_force(self, particle, theta):
        if self.total_mass == 0:
            return 0, 0
        
        # If this is an external node (leaf with one particle) and it's the same particle, skip
        if len(self.particles) == 1 and self.particles[0] == particle:
            return 0, 0
        
        # Calculate distance between particle and center of mass
        dx = self.center_of_mass.x - particle.position.x
        dy = self.center_of_mass.y - particle.position.y
        distance = math.sqrt(dx*dx + dy*dy)
        
        # If this is a leaf node or the distance is sufficient for approximation
        if not self.divided or (self.boundary.w * 2) / distance < theta:
            # Avoid division by zero and extreme forces at small distances
            if distance < SOFTENING:
                distance = SOFTENING
                
            # Calculate gravitational force
            f = G * particle.mass * self.total_mass / (distance * distance)
            
            # Resolve force into x and y components
            fx = f * dx / distance
            fy = f * dy / distance
            
            return fx, fy
        
        # Otherwise, recursively calculate forces from children
        fx, fy = 0, 0
        
        if self.northeast:
            fx_ne, fy_ne = self.northeast.calculate_force(particle, theta)
            fx += fx_ne
            fy += fy_ne
        
        if self.northwest:
            fx_nw, fy_nw = self.northwest.calculate_force(particle, theta)
            fx += fx_nw
            fy += fy_nw
        
        if self.southeast:
            fx_se, fy_se = self.southeast.calculate_force(particle, theta)
            fx += fx_se
            fy += fy_se
        
        if self.southwest:
            fx_sw, fy_sw = self.southwest.calculate_force(particle, theta)
            fx += fx_sw
            fy += fy_sw
        
        return fx, fy

def create_deterministic_galaxy(num_particles, center_x, center_y, radius=300, spiral_factor=0.1):
    """Create a deterministic galaxy-like distribution of particles"""
    particles = []
    
    # Create central bulge (30% of particles)
    bulge_count = int(num_particles * 0.3)
    for i in range(bulge_count):
        # Use deterministic distribution for the bulge
        # Distribute in a spiral pattern from center
        distance_factor = i / bulge_count
        r = radius / 5 * distance_factor
        angle = 2 * math.pi * i / bulge_count * 7  # 7 rotations for central bulge
        
        x = center_x + r * math.cos(angle)
        y = center_y + r * math.sin(angle)
        
        # Deterministic mass values
        mass = 50 + (50 * (1 - distance_factor))
        
        particle = Particle(x, y, mass)
        particles.append(particle)
    
    # Create spiral arms (70% of particles)
    spiral_count = num_particles - bulge_count
    arms = 2  # Number of spiral arms
    particles_per_arm = spiral_count // arms
    
    for arm in range(arms):
        base_angle = 2 * math.pi * arm / arms
        
        for i in range(particles_per_arm):
            # Distance from center (linearly distributed from inner to outer radius)
            distance_factor = i / particles_per_arm
            distance = radius * (0.2 + 0.8 * distance_factor)
            
            # Spiral angle based on distance from center
            angle = base_angle + spiral_factor * distance
            
            # Deterministic variation
            variation = 0.2 * math.sin(i * 0.1)
            angle += variation
            
            x = center_x + distance * math.cos(angle)
            y = center_y + distance * math.sin(angle)
            
            # Deterministic mass values
            mass = 1 + 9 * (1 - distance_factor)
            
            particle = Particle(x, y, mass)
            particles.append(particle)
    
    # Remaining particles (if odd number doesn't divide evenly into arms)
    remaining = spiral_count - (particles_per_arm * arms)
    for i in range(remaining):
        angle = 2 * math.pi * i / remaining
        distance = radius * 0.5
        
        x = center_x + distance * math.cos(angle)
        y = center_y + distance * math.sin(angle)
        
        particle = Particle(x, y, 5.0)  # Fixed mass
        particles.append(particle)
    
    # Add deterministic initial orbital velocities
    for p in particles:
        # Vector from center to particle
        dx = p.position.x - center_x
        dy = p.position.y - center_y
        distance = math.sqrt(dx*dx + dy*dy)
        
        if distance > 0:
            # Direction perpendicular to radial direction
            perp_x = -dy / distance
            perp_y = dx / distance
            
            # Orbital velocity (approximating balanced centripetal force)
            orbital_velocity = math.sqrt(0.1 * (distance + 100)) * 0.3
            
            p.velocity.x = perp_x * orbital_velocity
            p.velocity.y = perp_y * orbital_velocity
    
    return particles

def calculate_system_energy(particles):
    """Calculate the total energy of the system (kinetic + potential)"""
    energy = 0.0
    
    # Calculate potential energy
    for i in range(len(particles)):
        for j in range(i + 1, len(particles)):
            p1 = particles[i]
            p2 = particles[j]
            
            dx = p1.position.x - p2.position.x
            dy = p1.position.y - p2.position.y
            distance = math.sqrt(dx*dx + dy*dy)
            
            # Avoid division by zero
            if distance < SOFTENING:
                distance = SOFTENING
            
            # Gravitational potential energy
            energy -= G * p1.mass * p2.mass / distance
    
    # Calculate kinetic energy
    for p in particles:
        v_squared = p.velocity.x * p.velocity.x + p.velocity.y * p.velocity.y
        energy += 0.5 * p.mass * v_squared
    
    return energy

def advance_system(particles, theta, time_step, width, height):
    """Advance the n-body system by one time step using the quadtree"""
    # Create a fresh quadtree
    boundary = Rectangle(width / 2, height / 2, width / 2, height / 2)
    quadtree = QuadTree(boundary)
    
    # Insert all particles into the quadtree
    for particle in particles:
        quadtree.insert(particle)
    
    # Calculate and apply forces to all particles
    for particle in particles:
        fx, fy = quadtree.calculate_force(particle, theta)
        particle.apply_force(fx, fy)
    
    # Update all particles
    for particle in particles:
        particle.update(time_step)

def bench_quadtree_nbody(loops, num_particles, iterations, theta):
    # Initialize simulation space
    width = 1000
    height = 800
    
    # Create deterministic galaxy distribution
    particles = create_deterministic_galaxy(num_particles, width / 2, height / 2)
    
    # Calculate initial energy
    initial_energy = calculate_system_energy(particles)
    
    range_it = range(loops)
    t0 = pyperf.perf_counter()
    
    for _ in range_it:
        # Run simulation for specified iterations
        for _ in range(iterations):
            advance_system(particles, theta, TIME_STEP, width, height)
        
        # Calculate final energy
        final_energy = calculate_system_energy(particles)
    
    return pyperf.perf_counter() - t0

def add_cmdline_args(cmd, args):
    cmd.extend(("--iterations", str(args.iterations)))
    cmd.extend(("--particles", str(args.particles)))
    cmd.extend(("--theta", str(args.theta)))

if __name__ == '__main__':
    runner = pyperf.Runner(add_cmdline_args=add_cmdline_args)
    runner.metadata['description'] = "Quadtree N-body benchmark"
    
    runner.argparser.add_argument("--iterations",
                                  type=int, default=DEFAULT_ITERATIONS,
                                  help="Number of simulation steps per benchmark loop "
                                       "(default: %s)" % DEFAULT_ITERATIONS)
    
    runner.argparser.add_argument("--particles",
                                  type=int, default=DEFAULT_PARTICLES,
                                  help="Number of particles in the simulation "
                                       "(default: %s)" % DEFAULT_PARTICLES)
    
    runner.argparser.add_argument("--theta",
                                  type=float, default=DEFAULT_THETA,
                                  help="Barnes-Hut approximation threshold "
                                       "(default: %s)" % DEFAULT_THETA)
    
    args = runner.parse_args()
    runner.bench_time_func('quadtree_nbody', bench_quadtree_nbody,
                          args.particles, args.iterations, args.theta)
