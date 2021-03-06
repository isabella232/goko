//! See the paper for how this works

use hashbrown::HashMap;
use crate::covertree::CoverTreeReader;
use crate::plugins::*;

use super::categorical::*;
use super::dirichlet::*;

use serde::{Deserialize, Serialize};

use std::fmt;

use std::collections::VecDeque;

/// Computes a frequentist KL divergence calculation on each node the sequence touches.
pub struct BayesCategoricalTracker<D: PointCloud> {
    running_evidence: HashMap<NodeAddress, Categorical>,
    sequence_queue: VecDeque<Vec<(f32, NodeAddress)>>,
    sequence_count: usize,
    window_size: usize,
    prior_weight: f64,
    observation_weight: f64,
    reader: CoverTreeReader<D>,
}

impl<D: PointCloud> fmt::Debug for BayesCategoricalTracker<D> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "PointCloud {{ sequence_queue: {:?}, window_size: {} prior_weight: {}, observation_weight: {}, running_evidence: {:?}}}",
            self.sequence_queue, self.window_size, self.prior_weight, self.observation_weight, self.running_evidence,
        )
    }
}

impl<D: PointCloud> BayesCategoricalTracker<D> {
    /// Creates a new blank thing with capacity `size`, input 0 for unlimited.
    pub fn new(
        prior_weight: f64,
        observation_weight: f64,
        window_size: usize,
        reader: CoverTreeReader<D>,
    ) -> BayesCategoricalTracker<D> {
        BayesCategoricalTracker {
            running_evidence: HashMap::new(),
            sequence_queue: VecDeque::new(),
            sequence_count: 0,
            window_size,
            prior_weight,
            observation_weight,
            reader,
        }
    }

    /// Appends a tracker to this one,
    pub fn append(mut self, other: &Self) -> Self {
        for (k, v) in other.running_evidence.iter() {
            self.running_evidence
                .entry(*k)
                .and_modify(|e| e.merge(v))
                .or_insert_with(|| v.clone());
        }
        self.sequence_queue
            .extend(other.sequence_queue.iter().cloned());
        self
    }

    fn get_distro(&self, address: NodeAddress) -> Dirichlet {
        let mut prob = self
            .reader
            .get_node_plugin_and::<Dirichlet, _, _>(address, |p| p.clone())
            .unwrap();
        let total = prob.total();
        if total > self.window_size as f64 {
            prob.weight((total.ln() * self.window_size as f64) / total)
        }
        prob.weight(self.prior_weight);
        prob
    }

    fn add_trace_to_pdfs(&mut self, trace: &[(f32, NodeAddress)]) {
        let parent_address_iter = trace.iter().map(|(_, ca)| ca);
        let mut child_address_iter = trace.iter().map(|(_, ca)| ca);
        child_address_iter.next();
        for (parent, child) in parent_address_iter.zip(child_address_iter) {
            self.running_evidence
                .entry(*parent)
                .or_default()
                .add_child_pop(Some(*child), self.observation_weight);
        }
        let last = trace.last().unwrap().1;
        self.running_evidence
            .entry(last)
            .or_default()
            .add_child_pop(None, self.observation_weight);
    }

    fn remove_trace_from_pdfs(&mut self, trace: &[(f32, NodeAddress)]) {
        let parent_address_iter = trace.iter().map(|(_, ca)| ca);
        let mut child_address_iter = trace.iter().map(|(_, ca)| ca);
        child_address_iter.next();
        for (parent, child) in parent_address_iter.zip(child_address_iter) {
            let parent_evidence = self.running_evidence.get_mut(parent).unwrap();
            parent_evidence.remove_child_pop(Some(*child), self.observation_weight);
        }
        let last = trace.last().unwrap().1;
        self.running_evidence
            .get_mut(&last)
            .unwrap()
            .remove_child_pop(None, self.observation_weight);
    }

    /// Gives the probability vector for this
    pub fn prob_vector(&self, na: NodeAddress) -> Option<(Vec<(NodeAddress, f64)>, f64)> {
        self.reader
            .get_node_plugin_and::<Dirichlet, _, _>(na, |p| {
                let mut dir = p.clone();
                if let Some(e) = self.running_evidence.get(&na) {
                    dir.add_evidence(e)
                }
                dir.prob_vector()
            })
            .flatten()
    }

    /// Gives the probability vector for this
    pub fn evidence_prob_vector(&self, na: NodeAddress) -> Option<(Vec<(NodeAddress, f64)>, f64)> {
        self.running_evidence
            .get(&na)
            .map(|e| e.prob_vector())
            .flatten()
    }

    /// Adds an element to the trace
    pub fn add_path(&mut self, trace: Vec<(f32, NodeAddress)>) {
        self.add_trace_to_pdfs(&trace);
        self.sequence_count += 1;
        if self.window_size != 0 {
            self.sequence_queue.push_back(trace);

            if self.sequence_queue.len() > self.window_size {
                let oldest = self.sequence_queue.pop_front().unwrap();
                self.remove_trace_from_pdfs(&oldest);
            }
        }
    }

    /// The running categorical distributions
    pub fn running_evidence(&self) -> &HashMap<NodeAddress, Categorical> {
        &self.running_evidence
    }

    /// The lenght of the sequence
    pub fn sequence_len(&self) -> usize {
        if self.sequence_queue.is_empty() {
            self.sequence_count
        } else {
            self.sequence_queue.len()
        }
    }

    /// Gives the per-node KL divergence, with the node address
    pub fn all_node_kl(&self) -> Vec<(f64, NodeAddress)> {
        self.running_evidence()
            .iter()
            .map(|(address, sequence_pdf)| {
                let kl = self
                    .reader
                    .get_node_plugin_and::<Dirichlet, _, _>(*address, |p| {
                        p.posterior_kl_divergence(sequence_pdf).unwrap()
                    })
                    .unwrap();
                (kl, *address)
            })
            .collect()
    }

    /// A set of stats for the sequence that are helpful.
    pub fn kl_div_stats(&self) -> KLDivergenceStats {
        let mut max = f64::MIN;
        let mut min = f64::MAX;
        let mut nz_count = 0;
        let mut moment1_nz = 0.0;
        let mut moment2_nz = 0.0;
        self.all_node_kl().iter().for_each(|(kl, _address)| {
            if *kl > 1.0e-10 {
                moment1_nz += kl;
                moment2_nz += kl * kl;
                if max < *kl {
                    max = *kl;
                }
                if *kl < min {
                    min = *kl;
                }

                nz_count += 1;
            }
        });
        KLDivergenceStats {
            max,
            min,
            nz_count,
            moment1_nz,
            moment2_nz,
            sequence_len: self.sequence_len(),
        }
    }

    /// A set of stats for the sequence that are helpful.
    pub fn fractal_dim_stats(&self) -> FractalDimStats {
        let mut layer_totals: Vec<u64> = vec![0; self.reader.len()];
        let mut layer_node_counts = vec![Vec::<usize>::new(); self.reader.len()];
        let parameters = self.reader.parameters();
        self.all_node_kl().iter().for_each(|(_kl, address)| {
            layer_totals[parameters.internal_index(address.0)] += 1;
            layer_node_counts[parameters.internal_index(address.0)].push(
                self.reader
                    .get_node_and(*address, |n| n.coverage_count())
                    .unwrap(),
            );
        });
        let weighted_layer_totals: Vec<f32> = layer_node_counts
            .iter()
            .map(|counts| {
                let max: f32 = *counts.iter().max().unwrap_or(&1) as f32;
                counts.iter().fold(0.0, |a, c| a + (*c as f32) / max)
            })
            .collect();
        FractalDimStats {
            sequence_len: self.sequence_len(),
            layer_totals,
            weighted_layer_totals,
        }
    }

    /// Easy access to the cover tree read head associated to this tracker
    pub fn reader(&self) -> &CoverTreeReader<D> {
        &self.reader
    }
}

/// Tracks the non-zero KL div (all KL divergences above 1e-10)
#[derive(Debug,Serialize, Deserialize)]
pub struct KLDivergenceStats {
    /// The maximum non-zero KL divergence
    pub max: f64,
    /// The minimum non-zero KL divergence
    pub min: f64,
    /// The number of nodes that have a non-zero divergence
    pub nz_count: u64,
    /// The first moment, use this with the `nz_count` to get the mean
    pub moment1_nz: f64,
    /// The second moment, use this with the `nz_count` and first moment to get the variance
    pub moment2_nz: f64,
    /// The number of sequence elements that went into calculating this stat. This is not the total lenght
    /// We can drop old sequence elements
    pub sequence_len: usize,
}

/// Stats that let you compute the fractal dim of the query dataset wrt the base covertree
#[derive(Debug,Serialize, Deserialize)]
pub struct FractalDimStats {
    /// The number of sequence elements that went into calculating this stat. This is not the total lenght
    /// We can drop old sequence elements
    pub sequence_len: usize,
    /// The number of nodes per layer this sequence touches
    pub layer_totals: Vec<u64>,
    /// The number of nodes per layer this sequence touches
    pub weighted_layer_totals: Vec<f32>,
}


